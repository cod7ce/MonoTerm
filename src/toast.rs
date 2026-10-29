use crate::{appearance::Palette, ui};
use gpui::{
    App, Context, Entity, Global, Render, SharedString, Subscription, Task, WeakEntity, Window,
    WindowId, div, prelude::*, px, rgb,
};
use std::{collections::HashMap, time::Duration};

#[derive(Default)]
struct ToastWindows(HashMap<WindowId, WeakEntity<Toast>>);
impl Global for ToastWindows {}

pub(crate) struct Toast {
    message: Option<SharedString>,
    palette: Palette,
    dismiss: Option<Task<()>>,
    _release: Subscription,
}

/// Mount once at the end of each window's root, outside scrollable content.
pub(crate) fn new(window: &Window, palette: Palette, cx: &mut App) -> Entity<Toast> {
    let id = window.window_handle().window_id();
    let toast = cx.new(|cx| Toast {
        message: None,
        palette,
        dismiss: None,
        _release: cx.on_release(move |_, cx| {
            cx.default_global::<ToastWindows>().0.remove(&id);
        }),
    });
    cx.default_global::<ToastWindows>()
        .0
        .insert(id, toast.downgrade());
    toast
}

/// Show a transient message in the caller's window without moving keyboard focus.
pub(crate) fn show(message: impl Into<SharedString>, window: &Window, cx: &mut App) {
    let toast = cx
        .try_global::<ToastWindows>()
        .and_then(|windows| windows.0.get(&window.window_handle().window_id()))
        .and_then(WeakEntity::upgrade);
    if let Some(toast) = toast {
        toast.update(cx, |toast, cx| {
            toast.message = Some(message.into());
            // Replacing the task cancels the previous deadline, even for identical text.
            toast.dismiss = Some(cx.spawn(async move |toast, cx| {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                if let Err(error) = toast.update(cx, |toast, cx| {
                    toast.message = None;
                    cx.notify();
                }) {
                    eprintln!("Dismiss toast: {error}");
                }
            }));
            cx.notify();
        });
    }
}

pub(crate) fn layer(toast: &Entity<Toast>, palette: Palette, cx: &mut App) -> gpui::Div {
    toast.update(cx, |toast, _| toast.palette = palette);
    div()
        .absolute()
        .top(px(10.0))
        .left_0()
        .right_0()
        .px(px(16.0))
        .flex()
        .justify_center()
        .child(toast.clone())
}

impl Render for Toast {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        div()
            .max_w_full()
            .when_some(self.message.clone(), |layer, message| {
                layer.child(
                    div()
                        .max_w_full()
                        .h(px(24.0))
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .font_family(".SystemUIFont")
                        .text_size(px(12.0))
                        .text_color(rgb(p.text))
                        .child(
                            div()
                                .size(px(14.0))
                                .flex()
                                .flex_shrink_0()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .bg(rgb(if p.dark {
                                    ui::FOLLOWING
                                } else {
                                    ui::FOLLOWING_LIGHT
                                }))
                                .child(
                                    ui::icon("check-thick")
                                        .size(px(10.0))
                                        .text_color(rgb(p.shell)),
                                ),
                        )
                        // The CJK ink of the system font sits half a pixel below its
                        // line-box center, so nudge it onto the badge's center line.
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .relative()
                                .top(px(-0.5))
                                .child(message),
                        ),
                )
            })
    }
}
