use cosmic::desktop::fde::IconSource;

/// A library entry owned by Hearthdeck, not the desktop shell.
#[derive(Clone, Debug)]
pub struct LibraryItem {
    pub id: String,
    pub name: String,
    pub exec: Option<String>,
    pub icon: IconSource,
    pub categories: Vec<String>,
    pub wm_class: Option<String>,
}

impl Default for LibraryItem {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            exec: None,
            icon: IconSource::Name(String::new()),
            categories: Vec::new(),
            wm_class: None,
        }
    }
}
