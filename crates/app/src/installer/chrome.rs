//! The window the setup, uninstall and data-moving flows share: a title bar,
//! a rail with the logo and the steps, and a panel for the current page.

use gpui_kit::assets::IconName;
use gpui_kit::component::TitleBar;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Bounds, Div, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, Styled as _, TitlebarOptions, WindowBounds,
    WindowControlArea, WindowOptions, div, px, relative, size,
};
use rust_i18n::t;

use crate::theme;
use crate::ui::widgets::{icon, logo_mark, panel};

/// A fixed-size window centred on the screen. Its title bar is drawn by
/// `frame`, so there is nothing to maximise.
pub fn window_options(title: SharedString, cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(theme::WIZARD_WIDTH), px(theme::WIZARD_HEIGHT)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(title),
            ..TitleBar::title_bar_options()
        }),
        is_resizable: false,
        ..Default::default()
    }
}

fn title_bar(title: SharedString) -> Div {
    div()
        .relative()
        .flex()
        .items_center()
        .justify_between()
        .flex_none()
        .h(theme::WIZARD_TITLEBAR_HEIGHT)
        .pl_4()
        .child(
            div()
                .absolute()
                .inset_0()
                .window_control_area(WindowControlArea::Drag),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(theme::text_tertiary())
                .child(title),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .h_full()
                .w(theme::WIZARD_CLOSE_WIDTH)
                .hover(|style| style.bg(theme::hover()))
                .window_control_area(WindowControlArea::Close)
                .child(icon(IconName::X, 14., theme::text_secondary())),
        )
}

fn step_dot(done: bool, current: bool) -> Div {
    let dot = div()
        .flex()
        .items_center()
        .justify_center()
        .flex_none()
        .size(px(theme::STEP_DOT))
        .rounded_full();

    if done {
        dot.bg(theme::accent())
            .child(icon(IconName::Check, theme::STEP_CHECK, theme::on_accent()))
    } else if current {
        dot.border_2().border_color(theme::accent())
    } else {
        dot.border_1().border_color(theme::text_tertiary())
    }
}

/// The logo with the app name and version, stacked.
pub fn rail_header() -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(logo_mark(theme::WIZARD_LOGO))
        .child(
            div()
                .pt_3()
                .text_size(px(20.))
                .font_weight(FontWeight::BOLD)
                .font_family(theme::FONT_DISPLAY)
                .text_color(theme::text_primary())
                .child("Sonear"),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(theme::text_tertiary())
                .child(
                    t!(
                        "installer.rail_version",
                        version = env!("CARGO_PKG_VERSION")
                    )
                    .into_owned(),
                ),
        )
}

/// One row per step, with the steps before `current` ticked.
pub fn step_list(steps: &[SharedString], current: usize) -> Div {
    let list = steps.iter().enumerate().map(|(index, label)| {
        let color = if index <= current {
            theme::text_primary()
        } else {
            theme::text_tertiary()
        };

        div()
            .flex()
            .items_center()
            .gap_3()
            .child(step_dot(index < current, index == current))
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(color)
                    .when(index == current, |this| {
                        this.font_weight(FontWeight::MEDIUM)
                    })
                    .child(label.clone()),
            )
    });

    div().flex().flex_col().gap_3().children(list)
}

fn rail(steps: &[SharedString], current: usize) -> Div {
    panel()
        .w(theme::WIZARD_RAIL_WIDTH)
        .flex_none()
        .p_6()
        .gap_1()
        .child(rail_header())
        .child(div().flex_1())
        .child(step_list(steps, current))
}

/// The whole window: title bar, rail and `page`. With no `steps` the rail
/// shows only the logo.
pub fn frame(
    title: impl Into<SharedString>,
    steps: &[SharedString],
    current: usize,
    page: impl IntoElement,
) -> Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .bg(theme::window())
        .font_family(theme::FONT)
        .text_color(theme::text_primary())
        .child(title_bar(title.into()))
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .gap(theme::PANEL_GAP)
                .px(theme::PANEL_GAP)
                .pb(theme::PANEL_GAP)
                .child(rail(steps, current))
                .child(page),
        )
}

/// One page: a title, a body that takes the free space, and a footer of buttons.
pub fn page(
    title: impl Into<SharedString>,
    body: impl IntoElement,
    footer: impl IntoElement,
) -> Div {
    panel()
        .flex_1()
        .min_w_0()
        .p(px(theme::WIZARD_PADDING))
        .gap_4()
        .child(
            div()
                .text_size(px(theme::WIZARD_TITLE_SIZE))
                .font_weight(FontWeight::BOLD)
                .font_family(theme::FONT_DISPLAY)
                .text_color(theme::text_primary())
                .child(title.into()),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .gap_3()
                .child(body),
        )
        .child(div().flex().justify_end().gap_2().child(footer))
}

/// A page whose title sits under a large icon, for the last step.
pub fn done_page(
    title: impl Into<SharedString>,
    body: impl IntoElement,
    footer: impl IntoElement,
) -> Div {
    page(
        title,
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(icon(
                IconName::CircleCheck,
                theme::WIZARD_STATUS_ICON,
                theme::accent(),
            ))
            .child(body),
        footer,
    )
}

/// A thin bar that fills with the accent colour as `fraction` goes from 0 to 1.
pub fn progress_bar(fraction: f32) -> Div {
    div()
        .w_full()
        .h(theme::PROGRESS_HEIGHT)
        .rounded_full()
        .bg(theme::hairline())
        .overflow_hidden()
        .child(
            div()
                .h_full()
                .w(relative(fraction.clamp(0., 1.)))
                .rounded_full()
                .bg(theme::accent()),
        )
}

/// A line of secondary text.
pub fn text(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(14.))
        .text_color(theme::text_secondary())
        .child(text.into())
}

/// A small line of tertiary text.
pub fn note(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.))
        .text_color(theme::text_tertiary())
        .child(text.into())
}

/// A coloured line, for warnings and errors.
pub fn message(text: impl Into<SharedString>, color: gpui_kit::Hsla) -> AnyElement {
    div()
        .text_size(px(13.))
        .text_color(color)
        .child(text.into())
        .into_any_element()
}
