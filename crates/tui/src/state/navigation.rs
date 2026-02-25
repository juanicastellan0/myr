#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pane {
    ConnectionWizard,
    SchemaExplorer,
    Results,
    QueryEditor,
    ProfileBookmarks,
}

impl Pane {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::ConnectionWizard => Self::SchemaExplorer,
            Self::SchemaExplorer => Self::Results,
            Self::Results => Self::QueryEditor,
            Self::QueryEditor => Self::ProfileBookmarks,
            Self::ProfileBookmarks => Self::SchemaExplorer,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SchemaLane {
    Databases,
    Tables,
    Columns,
}

impl SchemaLane {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Databases => Self::Tables,
            Self::Tables => Self::Columns,
            Self::Columns => Self::Databases,
        }
    }

    pub(crate) fn previous(self) -> Self {
        match self {
            Self::Databases => Self::Columns,
            Self::Tables => Self::Databases,
            Self::Columns => Self::Tables,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Databases => "Databases",
            Self::Tables => "Tables",
            Self::Columns => "Columns",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SchemaColumnViewMode {
    Compact,
    Full,
}

impl SchemaColumnViewMode {
    pub(crate) fn toggle(self) -> Self {
        match self {
            Self::Compact => Self::Full,
            Self::Full => Self::Compact,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Full => "full",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManagerLane {
    Profiles,
    Bookmarks,
}

impl ManagerLane {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Profiles => Self::Bookmarks,
            Self::Bookmarks => Self::Profiles,
        }
    }

    pub(crate) fn previous(self) -> Self {
        self.next()
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Profiles => "Profiles",
            Self::Bookmarks => "Bookmarks",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectionKey {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Msg {
    Quit,
    GoConnectionWizard,
    GoProfileBookmarkManager,
    ToggleHelp,
    NextPane,
    TogglePalette,
    TogglePerfOverlay,
    ToggleSafeMode,
    ToggleSchemaColumnView,
    Submit,
    CancelQuery,
    Navigate(DirectionKey),
    InvokeActionSlot(usize),
    InputChar(char),
    InsertNewline,
    Backspace,
    DeleteSelection,
    ClearInput,
    Connect,
    Tick,
}
