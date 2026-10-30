//! The console grid's source: which scope is on screen and the requests that
//! fill it. The pages themselves are held by [`crate::screens::library::console`].

use cosmic::app::Task;
use std::sync::Arc;

use crate::providers::daemon::retro_version_list;
use crate::screens::library::console;
use crate::screens::library::console::ConsoleScope;

use super::{App, CONSOLE_SEARCH_DEBOUNCE, Message, ROMM_PAGE_SIZE};

impl App {
    /// The RomM platform id of the console group in scope, when one is selected.
    pub(super) fn console_platform_id(&self) -> Option<i64> {
        self.cur_group
            .and_then(|index| self.config.sections.console_games.get(index))
            .and_then(|group| group.romm_platform_id())
    }

    /// The scope the console grid is showing: the tab's platform, narrowed by
    /// the search term and the facet selection.
    pub(super) fn console_scope(&self) -> ConsoleScope {
        ConsoleScope {
            platform: self.console_platform_id(),
            search: self.search_value.clone(),
            facets: self.facet_selection.clone(),
        }
    }

    /// Makes sure the scope on screen is what the grid draws, requesting its
    /// first page only if it has never been loaded.
    ///
    /// Whatever the scope holds is painted first - its cached pages, or
    /// nothing while it has never answered - which is what makes switching
    /// tabs instant and keeps the games of the query being replaced off the
    /// grid. A scope with a first page already out waits for it rather than
    /// asking twice.
    pub(super) fn ensure_console_scope(&mut self) -> Task<Message> {
        let scope = self.console_scope();
        self.refilter();
        match self.console.ensure(&scope) {
            console::Ensure::Cached => self.list_replaced(),
            console::Ensure::Loading => Task::none(),
            console::Ensure::Requested(generation) => self.fetch_console_page(scope, generation, 0),
        }
    }

    /// Records and performs one page request for `scope`, if the log wants it.
    ///
    /// Offset 0 opens a new load, abandoning one still in flight; a later page
    /// is refused while one is outstanding. The request is tagged with the
    /// generation, so the page that comes back is merged only if it describes
    /// the load the scope is still waiting for.
    pub(super) fn load_console_page(&mut self, scope: ConsoleScope, offset: u32) -> Task<Message> {
        let Some(generation) = self.console.begin(&scope, offset) else {
            return Task::none();
        };
        self.fetch_console_page(scope, generation, offset)
    }

    /// Performs the page request the console log already recorded.
    ///
    /// The tab, the facet selection and the search term all narrow the query at
    /// the source, so the grid is never limited to what the pages it already
    /// holds happened to carry.
    pub(super) fn fetch_console_page(
        &self,
        scope: ConsoleScope,
        generation: u64,
        offset: u32,
    ) -> Task<Message> {
        let platform_id = scope.platform;
        let search = scope.search.clone();
        let filters = scope.facets.clone();
        let client = self.daemon_client.clone();
        Task::perform(
            async move {
                client
                    .list_retro_records(
                        platform_id,
                        (!search.is_empty()).then_some(search.as_str()),
                        &filters,
                        ROMM_PAGE_SIZE,
                        offset,
                    )
                    .await
                    .map(|page| {
                        // The version lists are kept while the records still
                        // carry their metadata: the entry the grid draws has
                        // nowhere to put them.
                        let versions = page
                            .items
                            .iter()
                            .map(|record| (record.id.clone(), retro_version_list(record)))
                            .collect();
                        console::Page {
                            entries: page
                                .items
                                .into_iter()
                                .map(|record| Arc::new(record.into_library_item()))
                                .collect(),
                            versions,
                            total: page.total,
                            offset: page.offset,
                        }
                    })
                    .map_err(|error| error.to_string())
            },
            move |result| {
                cosmic::Action::App(Message::ConsolePageLoaded {
                    scope: scope.clone(),
                    generation,
                    result,
                })
            },
        )
    }

    /// Queues the background warm-up of every tab's unfiltered scope and issues
    /// the first requests under the model's inflight cap.
    ///
    /// The tabs are queued in the order they stand, the "all consoles" tab
    /// first: it is the landing view. Scopes that have answered or are loading
    /// are left out, so a repeat does not fetch them twice.
    pub(super) fn queue_console_prefetch(&mut self) -> Task<Message> {
        let platforms = std::iter::once(None).chain(
            self.config
                .sections
                .console_games
                .iter()
                .map(|group| group.romm_platform_id()),
        );
        self.console.queue_prefetch(platforms);
        self.pump_console_prefetch()
    }

    /// Issues the queued prefetches the log is ready to hand out, so walking the
    /// tab strip opens every console from cache instead of waiting.
    pub(super) fn pump_console_prefetch(&mut self) -> Task<Message> {
        let requests = self.console.next_prefetch();
        let fetches: Vec<_> = requests
            .into_iter()
            .map(|(scope, generation)| self.fetch_console_page(scope, generation, 0))
            .collect();
        Task::batch(fetches)
    }

    /// Fetches the console list, so the Console Games tabs match the user's
    /// actual RomM platforms rather than whatever was last persisted.
    pub(super) fn load_consoles(&self) -> Task<Message> {
        self.load_consoles_after(std::time::Duration::ZERO)
    }

    /// Fetches the console list after `delay`.
    pub(super) fn load_consoles_after(&self, delay: std::time::Duration) -> Task<Message> {
        let client = self.daemon_client.clone();
        Task::perform(
            async move {
                tokio::time::sleep(delay).await;
                client
                    .list_retro_consoles()
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| cosmic::Action::App(Message::ConsolesLoaded(result)),
        )
    }

    /// Fetches the filter facets the current console scope offers.
    pub(super) fn load_facets(&self) -> Task<Message> {
        let platform_id = self.console_platform_id();
        let client = self.daemon_client.clone();
        Task::perform(
            async move {
                client
                    .retro_facets(platform_id)
                    .await
                    .map_err(|error| error.to_string())
            },
            |result| cosmic::Action::App(Message::FacetsLoaded(result)),
        )
    }

    /// Re-queries the console grid once the user has stopped typing.
    ///
    /// The wait is a task rather than a subscription: the timeout that fires
    /// carries the term it was scheduled for, and only a term still in the field
    /// is searched, so a superseded wait needs no cancelling.
    pub(super) fn settle_console_search(&mut self) -> Task<Message> {
        let value = self.search_value.clone();
        Task::perform(
            // Built when the task runs, not when it is made: making the task
            // should not need a timer.
            async move { tokio::time::sleep(CONSOLE_SEARCH_DEBOUNCE).await },
            move |()| cosmic::Action::App(Message::ConsoleSearchSettled(value)),
        )
    }
}
