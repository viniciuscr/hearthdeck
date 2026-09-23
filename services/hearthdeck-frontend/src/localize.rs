// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: GPL-3.0-only

use i18n_embed::fluent::{FluentLanguageLoader, fluent_language_loader};
use i18n_embed::{DefaultLocalizer, LanguageLoader, Localizer};
use rust_embed::RustEmbed;
use std::sync::LazyLock;

#[derive(RustEmbed)]
#[folder = "i18n/"]
struct Localizations;

pub static LANGUAGE_LOADER: LazyLock<FluentLanguageLoader> = LazyLock::new(|| {
    let loader: FluentLanguageLoader = fluent_language_loader!();

    loader
        .load_fallback_language(&Localizations)
        .expect("Error while loading fallback language");

    loader
});

#[macro_export]
macro_rules! fl {
    ($message_id:literal) => {{
        i18n_embed_fl::fl!($crate::localize::LANGUAGE_LOADER, $message_id)
    }};

    // Tokens are captured and forwarded verbatim (`tt`), not re-parsed as
    // expressions. The macro this wraps is a proc macro, and a wrapped proc
    // macro that is fed re-printed `expr` metavariables loses its spans and
    // types — rust-analyzer then reports the generated `HashMap` argument as
    // mismatched on every one of these calls. Forwarding raw tokens keeps the
    // expansion analysable. rustc is unaffected either way.
    ($message_id:literal, $($args:tt)*) => {{
        i18n_embed_fl::fl!($crate::localize::LANGUAGE_LOADER, $message_id, $($args)*)
    }};
}

// Get the `Localizer` to be used for localizing this library.
pub fn localizer() -> Box<dyn Localizer> {
    Box::from(DefaultLocalizer::new(&*LANGUAGE_LOADER, &Localizations))
}

pub fn localize() {
    let localizer = localizer();
    let requested_languages = i18n_embed::DesktopLanguageRequester::requested_languages();

    if let Err(error) = localizer.select(&requested_languages) {
        eprintln!("Error while loading language for App List {}", error);
    }
}
