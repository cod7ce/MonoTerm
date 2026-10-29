//! English and Simplified Chinese text for app chrome, independent of terminal output.

use crate::appearance::{config_directory, write_atomic};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    #[serde(alias = "English")]
    English,
    #[serde(alias = "Chinese")]
    Chinese,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct InterfacePreferences {
    language: Language,
}

impl Language {
    pub const ALL: [Self; 2] = [Self::English, Self::Chinese];

    /// Native names remain recognizable when switching either direction.
    pub fn label(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Chinese => "简体中文",
        }
    }

    pub fn config_path() -> io::Result<PathBuf> {
        Ok(config_directory()?.join("interface.json"))
    }

    /// A missing file is normal. Other failures return a diagnostic alongside
    /// the English default, without overwriting the user's preferences.
    pub fn load() -> (Self, Option<String>) {
        match Self::config_path() {
            Ok(path) => Self::load_from(&path),
            Err(error) => (Self::default(), Some(error.to_string())),
        }
    }

    fn load_from(path: &Path) -> (Self, Option<String>) {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return (Self::default(), None);
            }
            Err(error) => {
                return (
                    Self::default(),
                    Some(format!("Could not read {}: {error}", path.display())),
                );
            }
        };
        match serde_json::from_slice::<InterfacePreferences>(&bytes) {
            Ok(preferences) => (preferences.language, None),
            Err(error) => (
                Self::default(),
                Some(format!("Could not decode {}: {error}", path.display())),
            ),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path().map_err(|error| error.to_string())?;
        self.save_to(&path)
    }

    fn save_to(&self, path: &Path) -> Result<(), String> {
        let mut bytes = serde_json::to_vec_pretty(&InterfacePreferences { language: *self })
            .map_err(|error| format!("Could not encode interface preferences: {error}"))?;
        bytes.push(b'\n');
        write_atomic(path, &bytes)
            .map_err(|error| format!("Could not save {}: {error}", path.display()))
    }

    /// English literals are the keys. Unknown text, brand names, code samples,
    /// and shortcut glyphs retain their source spelling.
    pub fn text(self, english: &'static str) -> &'static str {
        match self {
            Self::English => english,
            Self::Chinese => chinese(english).unwrap_or(english),
        }
    }

    /// Translate stored app notices and formatted UI labels at render time so
    /// an already-open session changes language immediately. Only known app
    /// templates are recognized; diagnostic details are preserved verbatim.
    /// Do not pass terminal output, protocol messages, or arbitrary paths here.
    pub fn dynamic_text(self, english: &str) -> String {
        if self == Self::English {
            return english.to_owned();
        }
        if let Some(translated) = chinese(english) {
            return translated.to_owned();
        }
        for (prefix, translated) in [
            ("Running · PID ", "运行中 · PID "),
            ("Disconnected · ", "已断开 · "),
            ("Disconnected: ", "已断开："),
            ("Connected: ", "已连接："),
            ("Could not start shell: ", "无法启动 shell："),
            ("Process error: ", "进程错误："),
            ("Resize failed: ", "调整大小失败："),
            ("Window effect: ", "窗口效果："),
            ("Open settings: ", "打开设置："),
            ("Open Herdr window: ", "打开 Herdr 窗口："),
        ] {
            if let Some(detail) = english.strip_prefix(prefix) {
                return format!("{translated}{detail}");
            }
        }
        if let Some(code) = english
            .strip_prefix("Exited (")
            .and_then(|value| value.strip_suffix(')'))
            .filter(|value| is_number(value))
        {
            return format!("已退出（{code}）");
        }
        if let Some(id) = english
            .strip_prefix("Workspace ")
            .filter(|value| is_number(value))
        {
            return format!("工作区 {id}");
        }
        if let Some(id) = english
            .strip_prefix("Pane ")
            .filter(|value| is_number(value))
        {
            return format!("窗格 {id}");
        }
        if let Some(count) = english
            .strip_suffix(" panes")
            .or_else(|| english.strip_suffix(" pane"))
            .filter(|value| is_number(value))
        {
            return format!("{count} 个窗格");
        }
        if let Some((tabs, panes)) = english.split_once(" tabs · ")
            && let Some(panes) = panes.strip_suffix(" panes")
            && is_number(tabs)
            && is_number(panes)
        {
            return format!("{tabs} 个标签页 · {panes} 个窗格");
        }
        if let Some(agent) = english.strip_suffix(" · kept") {
            return format!("{agent} · 已保留");
        }
        english.to_owned()
    }
}

fn is_number(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn chinese(english: &str) -> Option<&'static str> {
    Some(match english {
        // Settings categories and shared controls.
        "Settings" => "设置",
        "MonoTerm Settings" => "MonoTerm 设置",
        "Appearance" => "外观",
        "Window" => "窗口",
        "Terminal" => "终端",
        "Shortcuts" => "快捷键",
        "Integrations" => "集成",
        "Version" => "版本",
        "Get the latest release and view app version information." => {
            "获取最新版本，查看此应用的版本信息。"
        }
        "Native macOS terminal · GPUI + Ghostty" => "原生 macOS 终端 · GPUI + Ghostty",
        "Ready to check for updates" => "随时检查新版本",
        "Checking for updates…" => "正在检查更新…",
        "An update is available" => "发现新版本",
        "You’re up to date" => "当前已是最新版本",
        "Preparing to restart…" => "正在准备重启…",
        "Updates are disabled in test windows" => "测试窗口中已停用更新",
        "In-app updates are unavailable here" => "当前运行方式不支持应用内更新",
        "The update could not complete" => "更新尚未完成",
        "Available version" => "可用版本",
        "Check for updates" => "检查更新",
        "Releases & downloads" => "发布记录与下载",
        "Check and install updates in the normally launched application." => {
            "请在正常启动的应用中检查和安装更新。"
        }
        "Get a configured application package from Releases & downloads." => {
            "请从“发布记录与下载”获取配置完整的应用安装包。"
        }
        "Updates are checked manually and verified with Sparkle. Automatic checks and background downloads are off." => {
            "通过 Sparkle 手动检查更新并验证更新包签名。自动检查与后台下载默认关闭。"
        }
        "Installing an update restarts MonoTerm and closes local terminal sessions. Finish running jobs first. Herdr sessions keep running." => {
            "安装更新会重启 MonoTerm 并关闭本地终端会话，请先完成正在运行的任务。Herdr 会话会继续运行。"
        }
        "On" => "开",
        "Off" => "关",
        "Language" => "语言",
        "Chinese" | "Simplified Chinese" => "简体中文",
        "Changes the interface language in both windows." => "更改两个窗口的界面语言。",
        "Appearance & preferences" => "外观与偏好设置",
        "Make your terminal feel at home." => "让终端更合你的心意。",
        "Display mode" => "显示模式",
        "System" => "跟随系统",
        "Dark" => "深色",
        "Light" => "浅色",
        "Accent color" => "强调色",
        "Blue" => "蓝色",
        "Violet" => "紫色",
        "Pink" => "粉色",
        "Coral" => "珊瑚色",
        "Amber" => "琥珀色",
        "Sage" => "鼠尾草绿",
        "Ocean" => "海蓝色",
        "Forest" => "森林绿",
        "Mint" => "薄荷绿",
        "Paste hex" => "粘贴色值",
        "Copy a color in #RRGGBB format, then choose Paste hex." => {
            "请复制 #RRGGBB 格式的颜色，然后选择“粘贴色值”。"
        }
        "Surface hue" => "背景色相",
        "Saturation" => "饱和度",
        "Dark lightness" => "深色亮度",
        "Surface tint and accent are independent. Changes apply to both windows." => {
            "背景色调与强调色分别设置，修改会应用到两个窗口。"
        }
        "Accent tints the sidebar and surround, and highlights selected tabs and controls." => {
            "强调色用于侧栏和外围背景，并突出选中的标签页与控件。"
        }
        "Reset colors" => "重置颜色",
        "Set the atmosphere and make room for your work." => "调整窗口氛围，为工作腾出空间。",
        "Transparency" => "透明度",
        "Window blur" => "窗口模糊",
        "Main pane glass" => "主窗格玻璃效果",
        "Apply transparency to the shell and its toolbar." => "将透明度应用到 Shell 及其工具栏。",
        "Dismiss" => "关闭提示",
        "Light mode keeps surfaces opaque." => "浅色模式下背景保持不透明。",
        "Tab sidebar" => "标签页侧边栏",
        "This session · shown when the window has room." => "仅本次会话 · 窗口宽度足够时显示。",
        "Zen mode" => "禅模式",
        "This session · hide chrome and focus on the terminal." => {
            "仅本次会话 · 隐藏界面控件，专注于终端。"
        }
        "Shell and toolbar use this transparency; the surround is 30% less transparent. 0% is solid. Text stays opaque." => {
            "Shell 和工具栏使用此透明度，外围透明度降低 30%。0% 为不透明，文字始终不透明。"
        }
        "Opacity, blur, and glass are available in dark mode. Your values are kept."
        | "Transparency, blur, and glass are available in dark mode. Your values are kept." => {
            "透明度、模糊和玻璃效果仅在深色模式下可用，设置值会保留。"
        }
        "Comfortable type, with a preview at the actual size." => {
            "调整舒适的字体，并查看实际大小预览。"
        }
        "Font family" => "字体",
        "Saved for every pane, including new terminals." => {
            "保存并应用于所有窗格，包括新建的终端。"
        }
        "Font size" => "字号",
        "Applies to every pane in this session." => "应用于本次会话的所有窗格。",
        "10–28 px · starts at 15 px on launch" => "10–28 px · 启动时默认为 15 px",
        "Reset to 15 px" => "重置为 15 px",
        "MONOSPACE PREVIEW" => "等宽字体预览",
        "Font size is temporary. Appearance preferences are saved separately." => {
            "字号仅在本次会话有效，外观偏好设置会单独保存。"
        }
        "Font family is saved. Font size applies to this session." => {
            "字体选择会保存，字号仅在本次会话有效。"
        }
        "Could not save appearance. Changes are temporary." => "无法保存外观，修改仅临时生效。",
        "Could not save preferences. Changes are temporary." => {
            "无法保存偏好设置，修改仅临时生效。"
        }
        "Preferences could not be loaded. Defaults are in use." => {
            "无法加载偏好设置，已使用默认值。"
        }
        "Settings could not be applied or saved. Check the terminal status." => {
            "无法应用或保存设置，请查看终端状态。"
        }
        "Appearance saved automatically." => "外观设置已自动保存。",
        "Preferences saved automatically." => "偏好设置已自动保存。",
        "Appearance defaults" => "恢复默认外观",

        // Shortcut descriptions, tooltips, and native menus use exact English keys.
        "A reference for the terminal window. Bindings are fixed." => {
            "终端窗口快捷键参考，按键绑定不可修改。"
        }
        "Tabs & panes" => "标签页与窗格",
        "New tab" | "New Tab" => "新建标签页",
        "Close pane" | "Close Pane" => "关闭窗格",
        "Close tab" | "Close Tab" => "关闭标签页",
        "Next / previous tab" => "下一个／上一个标签页",
        "Split right / down" => "向右／向下分屏",
        "Next / previous pane" => "下一个／上一个窗格",
        "Restart active pane" => "重启活动窗格",
        "View & editing" => "视图与编辑",
        "Increase / decrease font" => "增大／减小字号",
        "Reset font size" => "重置字号",
        "Copy / paste" => "复制／粘贴",
        "Select all" | "Select All" => "全选",
        "Application" => "应用",
        "Open settings" => "打开设置",
        "Close settings" => "关闭设置",
        "Quit MonoTerm" => "退出 MonoTerm",
        "Edit" => "编辑",
        "Copy" => "复制",
        "Paste" => "粘贴",
        "View" => "视图",
        "Split Right" => "向右分屏",
        "Split Down" => "向下分屏",
        "Next Pane" => "下一个窗格",
        "Previous Pane" => "上一个窗格",
        "Toggle Zen Mode" => "切换禅模式",
        "Toggle Sidebar" => "切换侧边栏",
        "New local tab / return to local" => "新建本地标签页／返回本地",
        "Close Herdr window / detach client" => "关闭 Herdr 窗口／断开客户端",
        "Toggle sidebar · ⌘B" => "切换侧边栏 · ⌘B",
        "New tab · ⌘T" => "新建标签页 · ⌘T",
        "Split right · ⌘D" => "向右分屏 · ⌘D",
        "Split down · ⌘⇧D" => "向下分屏 · ⌘⇧D",
        "Zen mode · ⌘⇧Z" => "禅模式 · ⌘⇧Z",
        "Settings · ⌘," => "设置 · ⌘,",
        "Restart pane · ⌘⇧R" => "重启窗格 · ⌘⇧R",

        // Main window, Herdr client integration, and static status messages.
        "WORKSPACES" => "工作区",
        "TABS" => "标签页",
        "Customize tab" => "自定义标签页",
        "Customize tab…" => "自定义标签页…",
        "Title" => "标题",
        "Icon" => "图标",
        "Default" => "默认",
        "Code" => "代码",
        "Server" => "服务器",
        "Database" => "数据库",
        "Globe" => "地球",
        "Star" => "星标",
        "Lightning" => "闪电",
        "Branch" => "分支",
        "Save" => "保存",
        "Cancel" => "取消",
        "Restore default" => "恢复默认",
        "Leave blank to use the automatic title." => "留空以使用自动标题。",
        "Applies to this tab for the current session." => "应用于此标签页，仅本次会话有效。",
        "MonoTerm · Local workspace" => "MonoTerm · 本地工作区",
        "Pane" => "窗格",
        "Pane {id}" => "窗格 {id}",
        "{} pane{}" => "{} 个窗格{}",
        "{} tabs · {} panes" => "{} 个标签页 · {} 个窗格",
        "No session" => "无会话",
        "Starting…" => "正在启动…",
        "Limit reached: 12 tabs / 48 panes." => "已达上限：12 个标签页／48 个窗格。",
        "Limit reached: 8 panes per tab / 48 panes total." => {
            "已达上限：每个标签页 8 个窗格／共 48 个窗格。"
        }
        "Close a tab before opening another workspace." => "请先关闭一个标签页，再打开其他工作区。",
        "Herdr agents — MonoTerm" => "Herdr 智能体 — MonoTerm",
        "Herdr agents" => "Herdr 智能体",
        "SPACES" | "spaces" => "空间",
        "AGENTS" | "agents" => "智能体",
        "new" => "新建",
        "menu" => "菜单",
        "New workspace" => "新建工作区",
        "Back" => "后退",
        "Forward" => "前进",
        "No matching agents" => "没有匹配的智能体",
        "No spaces available" => "暂无空间",
        "Herdr follow" => "Herdr 随航",
        "Enable Herdr follow" => "启用 Herdr 随航",
        "Automatically open Herdr window" => "自动打开 Herdr 窗口",
        "On launch, enabling follow or clicking the Herdr icon. Existing windows stay open." => {
            "启动、启用随航或点击 Herdr 图标时生效；已打开的窗口保持不变。"
        }
        "Following Herdr" => "正在随航",
        "Pinned" => "已固定",
        "Resume" => "恢复随航",
        "Waiting for a Herdr agent" => "等待 Herdr 智能体",
        "Spaces and agents share the Herdr window. This window follows with local terminals." => {
            "空间与智能体显示在 Herdr 窗口中，当前窗口以本地终端随航。"
        }
        "Reuse a local shell and its splits for each selected agent." => {
            "为每个选中的智能体复用本地 shell 及其分屏。"
        }
        "Close unused companions" => "关闭未使用的随航终端",
        "After agent exit, close only untouched, idle local shells." => {
            "智能体退出后，仅关闭未使用且空闲的本地 shell。"
        }
        "Herdr client" => "Herdr 客户端",
        "Enable Herdr client" => "启用 Herdr 客户端",
        "Switch agents and use their live terminals in one reusable window." => {
            "切换智能体，并在同一个窗口中使用其真实终端。"
        }
        "Open the agent switcher on launch. Local terminals are preserved." => {
            "启动时打开智能体切换器。本地终端会保留。"
        }
        "Agent updates continue while local terminals are shown." => {
            "显示本地终端时，仍会持续更新智能体状态。"
        }
        "Choose an agent to open its live terminal." => "选择智能体以打开其真实终端。",
        "Agents appear here when Herdr detects them." => "Herdr 检测到的智能体会显示在这里。",
        "No agents available" => "暂无智能体",
        "Waiting for Herdr" => "等待 Herdr",
        "Waiting for the selected terminal" => "等待选中的终端",
        "Input cancelled because the selected terminal changed" => "选中的终端已更改，输入已取消",
        "Directory unavailable" => "目录不可用",
        "Return to local terminals" => "返回本地终端",
        "Open Herdr window" => "打开 Herdr 窗口",
        "No local terminals" => "暂无本地终端",
        "Press ⌘T to open a local terminal." => "按 ⌘T 打开本地终端。",
        "Connection" => "连接",
        "Reconnect" => "重新连接",
        "Herdr source socket" => "Herdr 来源套接字",
        "Use the daemon's JSON socket. Its path determines the client surface endpoint." => {
            "使用守护进程的 JSON 套接字。其路径决定客户端显示端点。"
        }
        "Closing the Herdr window detaches this client. Agents and local terminals keep running." => {
            "关闭 Herdr 窗口仅断开此客户端，智能体与本地终端继续运行。"
        }
        "Idle" => "空闲",
        "Working" => "工作中",
        "Needs attention" => "等待处理",
        "Done" => "已完成",
        "Unknown" => "未知",
        "Paste path" => "粘贴路径",
        "Use default" => "使用默认值",
        "Copy an absolute socket path first." => "请先复制套接字的绝对路径。",
        "Use an absolute local Herdr socket path." => "请使用本地 Herdr 套接字的绝对路径。",
        "Connecting" | "Connecting…" => "正在连接…",
        "Changing pane focus" => "正在切换窗格",
        "Waiting for surface acknowledgement" => "等待终端画面确认",
        "Connected (inactive)" => "已连接（未激活）",
        "Herdr client and daemon versions are incompatible" => "Herdr 客户端与守护进程版本不兼容",
        "Connected" => "已连接",
        "Disconnected" => "已断开",
        "Paste exceeds this app's 4 MiB limit" => "粘贴内容超过此应用的 4 MiB 上限",
        "Input queue is full; wait and retry" => "输入队列已满，请稍后重试",
        "Session has exited; restart to open another shell" => "会话已退出，请重启以打开新的 shell",
        "Session is closed" => "会话已关闭",
        "Session input is closed" => "会话输入已关闭",
        "Paste exceeds this prototype's 4 MiB limit" => "粘贴内容超过此原型的 4 MiB 上限",
        "Input queue is full; wait for the process and retry" => {
            "输入队列已满，请等待进程处理后重试"
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "monoterm-locale-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn language_defaults_and_serialization_are_stable() {
        assert_eq!(Language::default(), Language::English);
        assert_eq!(
            serde_json::from_str::<InterfacePreferences>("{}")
                .unwrap()
                .language,
            Language::English
        );
        for (language, json) in [
            (Language::English, r#""english""#),
            (Language::Chinese, r#""chinese""#),
        ] {
            assert_eq!(serde_json::to_string(&language).unwrap(), json);
            assert_eq!(serde_json::from_str::<Language>(json).unwrap(), language);
        }
        assert!(serde_json::from_str::<Language>(r#""unknown""#).is_err());
        assert!(serde_json::from_str::<InterfacePreferences>(r#"{"language":null}"#).is_err());
    }

    #[test]
    fn preferences_round_trip_and_bad_files_do_not_get_repaired() {
        let directory = TestDirectory::new();
        let path = directory.0.join("nested/interface.json");
        assert_eq!(Language::load_from(&path), (Language::English, None));
        assert!(!path.exists());
        Language::Chinese.save_to(&path).unwrap();
        assert_eq!(Language::load_from(&path), (Language::Chinese, None));
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&fs::read(&path).unwrap()).unwrap(),
            serde_json::json!({"language": "chinese"})
        );
        Language::English.save_to(&path).unwrap();
        assert_eq!(Language::load_from(&path), (Language::English, None));
        for contents in ["{", r#"{"language":"unsupported"}"#] {
            fs::write(&path, contents).unwrap();
            let (language, diagnostic) = Language::load_from(&path);
            assert_eq!(language, Language::English);
            assert!(diagnostic.unwrap().starts_with("Could not decode "));
            assert_eq!(fs::read_to_string(&path).unwrap(), contents);
        }
        assert!(Language::Chinese.save_to(&directory.0).is_err());
        assert!(Language::load_from(&directory.0).1.is_some());
    }

    #[test]
    fn translation_preserves_english_shortcuts_and_unknown_text() {
        assert_eq!(Language::English.text("Settings"), "Settings");
        assert_eq!(Language::Chinese.text("Settings"), "设置");
        assert_eq!(Language::Chinese.text("Font family"), "字体");
        assert_eq!(Language::Chinese.text("New tab · ⌘T"), "新建标签页 · ⌘T");
        assert_eq!(Language::Chinese.text("⌃Tab / ⌃⇧Tab"), "⌃Tab / ⌃⇧Tab");
        assert_eq!(
            Language::Chinese.text("let answer = 42;"),
            "let answer = 42;"
        );
        let unknown = String::from("new diagnostic: /tmp/Settings");
        assert_eq!(Language::Chinese.dynamic_text(&unknown), unknown);
        assert_eq!(Language::Chinese.text("MonoTerm"), "MonoTerm");
    }

    #[test]
    fn dynamic_labels_translate_only_the_app_owned_parts() {
        let chinese = Language::Chinese;
        assert_eq!(chinese.dynamic_text("Connected"), "已连接");
        assert_eq!(
            chinese.dynamic_text("Running · PID 123"),
            "运行中 · PID 123"
        );
        assert_eq!(chinese.dynamic_text("Exited (1)"), "已退出（1）");
        assert_eq!(chinese.dynamic_text("Pane 12"), "窗格 12");
        assert_eq!(chinese.dynamic_text("1 pane"), "1 个窗格");
        assert_eq!(chinese.dynamic_text("2 panes"), "2 个窗格");
        assert_eq!(
            chinese.dynamic_text("2 tabs · 3 panes"),
            "2 个标签页 · 3 个窗格"
        );
        assert_eq!(chinese.dynamic_text("codex · kept"), "codex · 已保留");
        assert_eq!(
            chinese.dynamic_text("Disconnected · Connect to Herdr: /tmp/Settings.sock"),
            "已断开 · Connect to Herdr: /tmp/Settings.sock"
        );
        for text in [
            "Pane project",
            "several panes",
            "2 tabs · unknown panes",
            "Exited (error)",
        ] {
            assert_eq!(chinese.dynamic_text(text), text);
        }
        assert_eq!(
            Language::English.dynamic_text("2 tabs · 3 panes"),
            "2 tabs · 3 panes"
        );
    }
}
