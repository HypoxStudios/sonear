use gpui_kit::assets::IconName;
use gpui_kit::{
    AnyElement, FontWeight, IntoElement, ParentElement as _, SharedString, Styled as _, div, px,
};

use crate::theme;
use crate::ui::widgets::icon;

pub fn empty_state(
    name: IconName,
    title: impl Into<SharedString>,
    body: impl Into<SharedString>,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .size_full()
        .p_8()
        .child(icon(name, 30., theme::text_tertiary()))
        .child(
            div()
                .pt_2()
                .text_size(px(18.))
                .font_weight(FontWeight::BOLD)
                .font_family(theme::FONT_DISPLAY)
                .text_color(theme::text_primary())
                .child(title.into()),
        )
        .child(
            div()
                .max_w(px(420.))
                .text_center()
                .text_size(px(13.))
                .text_color(theme::text_secondary())
                .child(body.into()),
        )
        .into_any_element()
}
