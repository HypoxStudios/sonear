use std::time::Duration;

use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, FontWeight, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Task, Window,
    WindowBounds, WindowId, WindowKind, WindowOptions, div, point, px, size,
};
use rust_i18n::t;

use super::widgets::{accent_button, ghost_button, logo_mark};
use crate::audio::OutputDevice;
use crate::store::AppStore;
use crate::theme;

/// The popup closes by itself after this long, which counts as keeping the current device.
const OPEN_FOR: Duration = Duration::from_secs(20);

/// Opens the popup in the bottom-right corner of the primary display's usable
/// area, above the mini player when `above_mini` is set.
pub fn open(
    store: Entity<AppStore>,
    device: OutputDevice,
    current_name: String,
    above_mini: bool,
    cx: &mut App,
) -> anyhow::Result<AnyWindowHandle> {
    let area = cx.primary_display().map(|display| display.visible_bounds());
    let (x, y) = match area {
        Some(area) => {
            let lift = if above_mini {
                theme::MINI_HEIGHT + theme::OUTPUT_POPUP_ABOVE_MINI
            } else {
                0.
            };

            (
                f32::from(area.origin.x) + f32::from(area.size.width)
                    - theme::OUTPUT_POPUP_WIDTH
                    - theme::MINI_MARGIN,
                f32::from(area.origin.y) + f32::from(area.size.height)
                    - theme::OUTPUT_POPUP_HEIGHT
                    - theme::MINI_MARGIN
                    - lift,
            )
        }
        None => (theme::MINI_MARGIN, theme::MINI_MARGIN),
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(x), px(y)),
            size(
                px(theme::OUTPUT_POPUP_WIDTH),
                px(theme::OUTPUT_POPUP_HEIGHT),
            ),
        ))),
        titlebar: None,
        kind: WindowKind::PopUp,
        focus: false,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    let (handle, _) = gpui_kit::open_window(options, cx, |window, cx| {
        if let Some(hwnd) = crate::platform_window::hwnd(window) {
            crate::platform_window::make_borderless(hwnd);
        }

        cx.new(|cx| OutputPopup::new(store, device, current_name, window, cx))
    })?;

    Ok(handle)
}

pub struct OutputPopup {
    store: Entity<AppStore>,
    device: OutputDevice,
    current_name: String,
    _timer: Task<()>,
}

impl OutputPopup {
    fn new(
        store: Entity<AppStore>,
        device: OutputDevice,
        current_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let id: WindowId = window.window_handle().window_id();
        let weak = store.downgrade();

        cx.on_release(move |_, cx| {
            let _ = weak.update(cx, |store, _| store.output_popup_closed(id));
        })
        .detach();

        let timer = cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(OPEN_FOR).await;

            let _ = this.update_in(cx, |_, window, _| window.remove_window());
        });

        Self {
            store,
            device,
            current_name,
            _timer: timer,
        }
    }
}

impl Render for OutputPopup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.clone();
        let id = self.device.id.clone();
        let name = self.device.name.clone();

        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .child(logo_mark(theme::OUTPUT_POPUP_MARK))
            .child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text_primary())
                    .child(t!("output.popup.title")),
            );
        let body = div()
            .truncate()
            .text_size(px(13.))
            .text_color(theme::text_secondary())
            .child(t!("output.popup.body", name = name));
        let buttons = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                accent_button("output-switch", t!("output.popup.switch"), None, true).on_click(
                    cx.listener(move |_, _, window, cx| {
                        let id = id.clone();

                        window.remove_window();
                        store.update(cx, |store, cx| store.switch_output(Some(id), cx));
                    }),
                ),
            )
            .child(
                ghost_button(
                    "output-keep",
                    t!("output.popup.keep", name = self.current_name.as_str()),
                    true,
                )
                .flex_1()
                .min_w_0()
                .truncate()
                .on_click(|_, window, _| window.remove_window()),
            );

        div()
            .size_full()
            .flex()
            .flex_col()
            .justify_between()
            .p(px(theme::OUTPUT_POPUP_PADDING))
            .bg(theme::player())
            .border_1()
            .border_color(theme::hairline())
            .font_family(theme::FONT)
            .child(div().flex().flex_col().gap_2().child(header).child(body))
            .child(buttons)
    }
}
