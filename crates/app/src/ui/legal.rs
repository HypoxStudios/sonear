use gpui_kit::assets::IconName;
use gpui_kit::component::text::TextView;
use gpui_kit::{
    Div, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px, relative,
};
use rust_i18n::t;

use super::widgets::{icon, icon_button};
use crate::legal::LegalDoc;
use crate::site;
use crate::store::AppStore;
use crate::theme;

/// The agreement or policy over a scrim. Links inside it open in the browser.
pub fn legal_overlay(store: &Entity<AppStore>, doc: LegalDoc) -> impl IntoElement {
    let backdrop = store.clone();
    let close = store.clone();

    div()
        .id("legal-backdrop")
        .occlude()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::scrim())
        .on_click(move |_, _, cx| {
            backdrop.update(cx, |state, cx| state.close_legal(cx));
        })
        .child(
            div()
                .id("legal-dialog")
                .flex()
                .flex_col()
                .w(theme::LEGAL_WIDTH)
                .h(relative(theme::LEGAL_HEIGHT))
                .overflow_hidden()
                .rounded(theme::PANEL_RADIUS)
                .border_1()
                .border_color(theme::hairline())
                .bg(theme::panel())
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(header(doc, close))
                .child(
                    div()
                        .id("legal-scroll")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .px_6()
                        .pb_6()
                        .child(TextView::markdown(
                            match doc {
                                LegalDoc::Terms => "legal-terms",
                                LegalDoc::Privacy => "legal-privacy",
                            },
                            doc.text(),
                        )),
                ),
        )
}

fn header(doc: LegalDoc, close: Entity<AppStore>) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .flex_none()
        .px_6()
        .py_4()
        .border_b_1()
        .border_color(theme::hairline())
        .child(
            div()
                .text_size(px(20.))
                .font_weight(FontWeight::BOLD)
                .font_family(theme::FONT_DISPLAY)
                .text_color(theme::text_primary())
                .child(doc.title()),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .id("legal-website")
                        .flex()
                        .items_center()
                        .gap_1()
                        .cursor_pointer()
                        .tab_index(0)
                        .text_size(px(13.))
                        .text_color(theme::accent())
                        .hover(|style| style.text_color(theme::accent_hover()))
                        .focus_visible(|style| style.text_color(theme::text_primary()))
                        .child(t!("legal.on_website"))
                        .child(icon(IconName::ExternalLink, 12., theme::accent()))
                        .on_click(move |_, _, cx| cx.open_url(&site::legal(doc))),
                )
                .child(
                    icon_button(
                        "legal-close",
                        IconName::X,
                        16.,
                        30.,
                        theme::text_secondary(),
                        true,
                    )
                    .on_click(move |_, _, cx| {
                        close.update(cx, |state, cx| state.close_legal(cx));
                    }),
                ),
        )
}
