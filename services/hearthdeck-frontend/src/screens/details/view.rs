//! How the details screen is drawn.
//!
//! Layout: a full-bleed backdrop of the screenshot being previewed, washed so
//! the page text stays readable, with the big image card over it on the left
//! and the game's facts scrolling on the right. The action line holds every
//! interaction the screen has. The disc picker, when open, covers all of it as
//! the topmost layer.
//!
//! Every colour, size and spacing value is a token from [`crate::style`] or
//! COSMIC's theme; the widget classes this page uses are the shared ones.

use cosmic::Element;
use cosmic::cosmic_theme::Spacing;
use cosmic::iced::core::alignment::{Horizontal, Vertical};
use cosmic::iced::{Alignment, ContentFit, Length};
use cosmic::theme;
use cosmic::widget::{
    Id, autosize::autosize, button, column, container, icon, list, list::list_column, row,
    scrollable, space, text,
};

use crate::app::Message;
use crate::fl;
use crate::screens::details::{Details, Hero, focus};
use crate::style::{
    DETAILS_ACTION_WIDTH, DETAILS_FACT_LABEL_WIDTH, DETAILS_PICKER_WIDTH, DETAILS_PLAY_WIDTH,
    DETAILS_SHOT_ASPECT, DETAILS_SHOT_SIZE, DETAILS_TEXT_BODY, DETAILS_TEXT_CAPTION,
    DETAILS_TEXT_HEADER, DETAILS_TEXT_LARGE, DETAILS_TEXT_TITLE, ICON_BODY, ICON_SMALL,
    MENU_MAX_HEIGHT, action_bar, artwork_fit, backdrop_scrim, backdrop_wash, card_surface,
    details_action_bar_padding, details_action_height, details_hero_width,
    details_toggle_button_class, hero_card, modal_scrim, passthrough, primary_action_button_class,
};
use crate::widgets::application::{Selection, TileLabel, app_tile};
use crate::widgets::rail::rail;

pub fn view<'a>(details: &'a Details, window_width: f32) -> Element<'a, Message> {
    let Spacing {
        space_xxs,
        space_xs,
        space_s,
        space_m,
        space_l,
        space_xl,
        ..
    } = theme::spacing();
    // The cursor is read through `details.cursor()`, not off the stored focus,
    // so a row that disappeared (screenshots that failed to download) moves the
    // highlight back to Play instead of highlighting nothing.
    let cursor = details.cursor();

    // ---- Backdrop: the screenshot being previewed, full-bleed. Screenshots are
    // 16:9 and survive being scaled to the window; a portrait cover stretched
    // here only ever looked blurry, so with no screenshots the page stays a flat
    // surface and the art card carries what art there is. ----
    let backdrop: Element<'_, Message> = match details.backdrop_art() {
        Some(handle) => artwork_fit(handle, ContentFit::Cover, Length::Fill, Length::Fill),
        None => container(space::horizontal())
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
    };
    let wash: Element<'_, Message> = container(space::horizontal())
        .width(Length::Fill)
        .height(Length::Fill)
        .class(theme::Container::Custom(Box::new(backdrop_wash)))
        .into();
    let scrim: Element<'_, Message> = container(space::horizontal())
        .width(Length::Fill)
        .height(Length::Fill)
        .class(theme::Container::Custom(Box::new(backdrop_scrim)))
        .into();

    // ---- The big image: the screenshot being previewed, filling its card the
    // way hero art does. A game with no screenshots falls back to its cover —
    // contained, so an odd-shaped poster is not cropped. ----
    let hero: Element<'_, Message> = match details.hero() {
        Some(Hero::Screenshot(handle)) => {
            artwork_fit(handle, ContentFit::Cover, Length::Fill, Length::Fill)
        }
        Some(Hero::Cover(handle)) => {
            artwork_fit(handle, ContentFit::Contain, Length::Fill, Length::Fill)
        }
        None => container(icon::icon(
            icon::from_name("applications-games-symbolic").into(),
        ))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .into(),
    };
    // Hero card uses 16:9 aspect ratio to match screenshot proportions,
    // so horizontal screenshots display correctly without cropping.
    let hero_width = details_hero_width(window_width);
    let hero_height = hero_width * 9.0 / 16.0;
    let art_card = container(hero)
        .width(Length::Fixed(hero_width))
        .height(Length::Fixed(hero_height))
        .padding(space_s)
        .class(theme::Container::Custom(Box::new(hero_card)));

    // ---- Action line: every interaction the screen has, in one strip. ----
    // One builder for all of them, so heights, text sizes, icon sizes and
    // padding cannot drift apart; Play differs only by width and by using the
    // accent fill.
    let action_button = |target: focus::Focus,
                         icon_name: &'static str,
                         label: String,
                         width: f32,
                         class: theme::Button,
                         message: Message|
     -> Element<'_, Message> {
        button::custom(
            container(
                row![
                    icon::icon(icon::from_name(icon_name).size(ICON_BODY).into()),
                    text::body(label).size(DETAILS_TEXT_LARGE),
                ]
                .spacing(space_s)
                .align_y(Alignment::Center),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            // No colour of its own: the class below decides the label's colour,
            // and a bare container would overwrite it.
            .class(theme::Container::Custom(Box::new(passthrough))),
        )
        .id(focus::widget_id(target))
        .width(Length::Fixed(width))
        .height(Length::Fixed(details_action_height()))
        .padding(0)
        .class(class)
        .on_press(message)
        .into()
    };
    let favorite_icon = if details.favorite().unwrap_or(false) {
        "starred-symbolic"
    } else {
        "non-starred-symbolic"
    };
    let show_disc = details.versions.len() > 1;
    let mut action_line = row![
        action_button(
            focus::Focus::Back,
            "go-previous-symbolic",
            fl!("back"),
            DETAILS_ACTION_WIDTH,
            details_toggle_button_class(false),
            Message::CloseDetails,
        ),
        action_button(
            focus::Focus::Play,
            "media-playback-start-symbolic",
            fl!("play"),
            DETAILS_PLAY_WIDTH,
            primary_action_button_class(),
            Message::DetailsPlay,
        ),
        action_button(
            focus::Focus::Favorite,
            favorite_icon,
            fl!("favorite"),
            DETAILS_ACTION_WIDTH,
            details_toggle_button_class(details.favorite().unwrap_or(false)),
            Message::DetailsToggleFavorite,
        ),
    ]
    .spacing(space_m)
    .align_y(Alignment::Center);
    if show_disc {
        action_line = action_line.push(action_button(
            focus::Focus::Disc,
            "media-optical-symbolic",
            fl!("disc"),
            DETAILS_ACTION_WIDTH,
            details_toggle_button_class(details.disc_picker),
            Message::DetailsToggleDiscPicker,
        ));
    }
    action_line = action_line.push(action_button(
        focus::Focus::PlayLater,
        "alarm-symbolic",
        fl!("play-later"),
        DETAILS_ACTION_WIDTH,
        details_toggle_button_class(details.backlogged().unwrap_or(false)),
        Message::DetailsToggleBacklog,
    ));
    let action_strip = container(action_line)
        .width(Length::Fill)
        .padding(details_action_bar_padding())
        .align_x(Horizontal::Center)
        .class(theme::Container::Custom(Box::new(action_bar)));

    let chips: Element<'_, Message> = row(details
        .chips()
        .into_iter()
        .map(|label| {
            container(text(label).size(DETAILS_TEXT_CAPTION))
                .padding([space_xxs, space_s])
                .class(theme::Container::Custom(Box::new(card_surface)))
                .into()
        })
        .collect::<Vec<Element<'_, Message>>>())
    .spacing(space_xs)
    .align_y(Alignment::Center)
    .into();

    // The disc picker: a panel over the page, using the native list look so the
    // highlighted disc reads the same way as any other COSMIC list.
    let disc_picker: Element<'_, Message> = {
        let mut rows = list_column::<Message>();
        for (index, version) in details.versions.iter().enumerate() {
            let is_current = version.id == details.rom_id;
            let icon_name = if is_current {
                "checkbox-checked-symbolic"
            } else {
                "media-optical-symbolic"
            };
            rows = rows.add(
                list::button(
                    row![
                        icon::icon(icon::from_name(icon_name).size(ICON_SMALL).into()),
                        text::body(version.title.clone()).size(DETAILS_TEXT_BODY),
                    ]
                    .spacing(space_s)
                    .align_y(Alignment::Center),
                )
                .on_press(Message::DetailsSelectDisc(version.id))
                .selected(cursor == focus::Focus::DiscChoice(index)),
            );
        }

        container(
            container(
                column![
                    text::title3(fl!("disc-selection")).size(DETAILS_TEXT_HEADER),
                    autosize(
                        container(scrollable(rows.into_element())).padding(1),
                        Id::new("details-disc-autosize"),
                    )
                    .max_height(MENU_MAX_HEIGHT),
                ]
                .spacing(space_m),
            )
            .width(Length::Fixed(DETAILS_PICKER_WIDTH))
            .padding(space_m)
            .class(theme::Container::Custom(Box::new(card_surface))),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .class(theme::Container::Custom(Box::new(modal_scrim)))
        .into()
    };

    let fact_rows: Vec<Element<'_, Message>> = details
        .facts()
        .into_iter()
        .map(|(label, value)| {
            row![
                container(text::caption(label).size(DETAILS_TEXT_CAPTION))
                    .width(Length::Fixed(DETAILS_FACT_LABEL_WIDTH)),
                text::body(value)
                    .size(DETAILS_TEXT_BODY)
                    .width(Length::Fill),
            ]
            .spacing(space_m)
            .align_y(Alignment::Start)
            .into()
        })
        .collect();

    let summary: Element<'_, Message> = match details.summary() {
        Some(summary) => column![
            text::caption(fl!("about")).size(DETAILS_TEXT_CAPTION),
            scrollable(
                text::body(summary)
                    .size(DETAILS_TEXT_BODY)
                    .width(Length::Fill)
            )
            .height(Length::Fill)
            .scrollbar_width(0)
            .scroller_width(0),
        ]
        .spacing(space_xxs)
        .height(Length::Fill)
        .into(),
        None => space::vertical().height(Length::Fill).into(),
    };

    let mut right = column![
        text(details.title.clone())
            .size(DETAILS_TEXT_TITLE)
            .width(Length::Fill)
    ]
    .spacing(space_s)
    .width(Length::Fill)
    .height(Length::Fill);
    right = right.push(chips);
    // Both failure texts are the same shape of notice: a warning icon and the
    // reason, so neither needs a layout of its own.
    for reason in [details.error(), details.write_error()]
        .into_iter()
        .flatten()
    {
        right = right.push(
            row![
                icon::icon(icon::from_name("dialog-warning-symbolic").into()).size(ICON_SMALL),
                text::caption(fl!("details-unavailable", reason = reason.to_owned()))
                    .size(DETAILS_TEXT_CAPTION),
            ]
            .spacing(space_xs)
            .align_y(Alignment::Center),
        );
    }
    right = right.push(column(fact_rows).spacing(space_xs));
    right = right.push(summary);

    let shots: Element<'_, Message> = if details.screenshots.is_empty() {
        space::vertical().height(Length::Fixed(0.0)).into()
    } else {
        let cards: Vec<Element<'_, Message>> = details
            .screenshots
            .iter()
            .enumerate()
            .map(|(index, handle)| {
                // Bound to a local first: the `fl!` macro cannot infer the
                // type of a bare `index + 1` expression argument.
                let number = index + 1;
                let selection = if cursor == focus::Focus::Screenshot(index) {
                    Selection::Selected
                } else {
                    Selection::Unselected
                };
                app_tile(
                    focus::widget_id(focus::Focus::Screenshot(index)),
                    &fl!("screenshot", index = number),
                    handle.clone(),
                    // 16:9, its own shape, rather than cropped to a square.
                    DETAILS_SHOT_SIZE * DETAILS_SHOT_ASPECT,
                    DETAILS_SHOT_SIZE,
                    ContentFit::Cover,
                    1,
                    None,
                    Some(Message::DetailsSelectScreenshot(index)),
                    None,
                    TileLabel::OnArtwork,
                    selection,
                )
            })
            .collect();
        rail(
            Id::new("details-screenshots"),
            fl!("screenshots"),
            DETAILS_SHOT_SIZE,
        )
        .items(cards)
        .into()
    };

    // Scrolls on its own: a long summary or a long fact list must never push
    // the action line off the bottom of the screen.
    let info = scrollable(right)
        .height(Length::Fill)
        .scrollbar_width(0)
        .scroller_width(0);
    let body = row![art_card, info]
        .spacing(space_xl)
        .height(Length::Fill)
        .width(Length::Fill);
    let page = column![body, shots, action_strip]
        .spacing(space_m)
        .padding(space_l)
        .height(Length::Fill)
        .width(Length::Fill);

    let mut layers: Vec<Element<'_, Message>> = vec![backdrop, wash, scrim, page.into()];
    if details.disc_picker {
        layers.push(disc_picker);
    }

    container(cosmic::iced::widget::stack(layers))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
