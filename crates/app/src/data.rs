//! Loading helpers: show the cached copy first, then revalidate, and page
//! lists on demand without duplicate fetches.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use gpui_kit::{Context, Task};
use serde::Serialize;
use serde::de::DeserializeOwned;
use sonear_core::model::Page;
use sonear_core::{Core, Freshness, LibraryError};
use tokio::runtime::Handle;
use tokio::sync::mpsc;

pub type Fetch<T> = Box<
    dyn FnOnce(Arc<Core>) -> Pin<Box<dyn Future<Output = Result<T, LibraryError>> + Send>> + Send,
>;

/// Boxes a fetch so callers can write `fetch(|core| async move { ... })`.
pub fn fetch<T, F, Fut>(work: F) -> Fetch<T>
where
    F: FnOnce(Arc<Core>) -> Fut + Send + 'static,
    Fut: Future<Output = Result<T, LibraryError>> + Send + 'static,
{
    Box::new(move |core| Box::pin(work(core)))
}

/// What the app needs to start background work.
#[derive(Clone)]
pub struct Env {
    pub core: Arc<Core>,
    pub runtime: Handle,
}

#[derive(Debug, Clone)]
pub enum Load<T> {
    Loading,
    Ready { value: T },
    Failed(LibraryError),
}

impl<T> Load<T> {
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Ready { value, .. } => Some(value),
            _ => None,
        }
    }
}

/// One loaded thing. A failed refresh keeps showing what was already there.
pub struct Slot<T> {
    pub state: Load<T>,
}

impl<T> Default for Slot<T> {
    fn default() -> Self {
        Self {
            state: Load::Loading,
        }
    }
}

impl<T> Slot<T> {
    pub fn apply(&mut self, incoming: Load<T>) {
        if matches!(
            (&self.state, &incoming),
            (Load::Ready { .. }, Load::Failed(_))
        ) {
            return;
        }

        self.state = incoming;
    }

    pub fn value(&self) -> Option<&T> {
        self.state.value()
    }

    pub fn error(&self) -> Option<&LibraryError> {
        match &self.state {
            Load::Failed(error) => Some(error),
            _ => None,
        }
    }
}

/// Runs `work` on the runtime. With a `key`, the cached copy is delivered
/// first (and nothing is fetched when it is fresh), then the fetched value.
/// Drop the returned task to stop listening.
pub fn load<V: 'static, T>(
    env: &Env,
    key: Option<String>,
    work: Fetch<T>,
    cx: &mut Context<V>,
    apply: impl Fn(&mut V, Load<T>, &mut Context<V>) + 'static,
) -> Task<()>
where
    T: Serialize + DeserializeOwned + Send + 'static,
{
    let (sender, mut inbox) = mpsc::unbounded_channel::<Load<T>>();
    let core = env.core.clone();

    env.runtime.spawn(async move {
        if let Some(key) = &key
            && let Some(cached) = core.meta.get::<T>(key)
        {
            let _ = sender.send(Load::Ready {
                value: cached.value,
            });

            if cached.freshness == Freshness::Fresh {
                return;
            }
        }

        match work(core.clone()).await {
            Ok(value) => {
                if let Some(key) = &key {
                    core.meta.put(key, &value);
                }

                let _ = sender.send(Load::Ready { value });
            }
            Err(error) => {
                let _ = sender.send(Load::Failed(error));
            }
        }
    });

    cx.spawn(async move |this, cx| {
        while let Some(message) = inbox.recv().await {
            if this
                .update(cx, |view, cx| {
                    apply(view, message, cx);
                    cx.notify();
                })
                .is_err()
            {
                break;
            }
        }
    })
}

/// An accumulating, scroll-paged list.
pub struct Paged<T> {
    pub items: Vec<T>,
    pub total: Option<u32>,
    pub has_more: bool,
    pub loading: bool,
    pub error: Option<LibraryError>,
    pub first_loaded: bool,
    next_offset: u32,
    cursor: Option<String>,
}

impl<T> Default for Paged<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            total: None,
            has_more: false,
            loading: false,
            error: None,
            first_loaded: false,
            next_offset: 0,
            cursor: None,
        }
    }
}

/// How close to the end of the list (in rows) the next page is requested.
const PREFETCH_ROWS: usize = 12;

impl<T> Paged<T> {
    /// Shows the first page. A late revalidation does not undo pages the
    /// user has already scrolled through.
    pub fn set_first(&mut self, page: Page<T>) {
        let page_len = page.items.len();

        if self.first_loaded && self.items.len() > page_len {
            self.total = page.total.or(self.total);
            self.loading = false;

            return;
        }

        self.next_offset = page.offset + page_len as u32;
        self.items = page.items;
        self.total = page.total;
        self.has_more = page.has_more;
        self.cursor = page.cursor;
        self.first_loaded = true;
        self.loading = false;
        self.error = None;
    }

    pub fn append(&mut self, page: Page<T>) {
        self.next_offset = page.offset + page.items.len() as u32;
        self.items.extend(page.items);
        self.total = page.total.or(self.total);
        self.has_more = page.has_more;
        self.cursor = page.cursor;
        self.loading = false;
        self.error = None;
    }

    pub fn fail(&mut self, error: LibraryError) {
        self.loading = false;
        self.error = Some(error);
    }

    /// Whether the visible rows reach close enough to the end to ask for more.
    pub fn wants_more(&self, visible_end: usize) -> bool {
        self.first_loaded
            && self.has_more
            && !self.loading
            && self.error.is_none()
            && visible_end + PREFETCH_ROWS >= self.items.len()
    }

    /// Marks a fetch as started and returns where it continues. `None` when one
    /// is already running, which de-duplicates requests.
    pub fn begin_next(&mut self) -> Option<(u32, Option<String>)> {
        if self.loading {
            return None;
        }

        self.loading = true;
        self.error = None;

        Some((self.next_offset, self.cursor.clone()))
    }

    pub fn retry(&mut self) {
        self.error = None;
    }
}

/// Fetches one more page on the runtime and delivers it to `apply`.
pub fn load_page<V: 'static, R: Send + 'static>(
    env: &Env,
    work: impl FnOnce(Arc<Core>) -> Pin<Box<dyn Future<Output = Result<R, LibraryError>> + Send>>
    + Send
    + 'static,
    cx: &mut Context<V>,
    apply: impl FnOnce(&mut V, Result<R, LibraryError>, &mut Context<V>) + 'static,
) {
    let (sender, mut inbox) = mpsc::unbounded_channel();
    let core = env.core.clone();

    env.runtime.spawn(async move {
        let _ = sender.send(work(core).await);
    });

    cx.spawn(async move |this, cx| {
        if let Some(result) = inbox.recv().await {
            let _ = this.update(cx, |view, cx| {
                apply(view, result, cx);
                cx.notify();
            });
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(offset: u32, count: u32, total: u32) -> Page<u32> {
        Page {
            items: (offset..offset + count).collect(),
            total: Some(total),
            offset,
            has_more: offset + count < total,
            cursor: None,
        }
    }

    #[test]
    fn paging_accumulates_and_asks_only_near_the_end() {
        let mut list = Paged::default();

        assert!(!list.wants_more(0));

        list.set_first(page(0, 50, 120));

        assert!(!list.wants_more(10));
        assert!(list.wants_more(40));
        assert_eq!(list.begin_next(), Some((50, None)));
        assert_eq!(list.begin_next(), None);
        assert!(!list.wants_more(49));

        list.append(page(50, 50, 120));
        list.append(page(100, 20, 120));

        assert_eq!(list.items.len(), 120);
        assert!(!list.has_more);
        assert!(!list.wants_more(119));
    }

    #[test]
    fn a_failed_page_stops_asking_until_retried() {
        let mut list = Paged::default();

        list.set_first(page(0, 50, 120));
        list.begin_next();
        list.fail(LibraryError::Forbidden);

        assert!(!list.wants_more(49));

        list.retry();

        assert!(list.wants_more(49));
    }

    #[test]
    fn a_late_revalidation_keeps_scrolled_pages() {
        let mut list = Paged::default();

        list.set_first(page(0, 50, 120));
        list.append(page(50, 50, 120));
        list.set_first(page(0, 50, 121));

        assert_eq!(list.items.len(), 100);
        assert_eq!(list.total, Some(121));
    }

    #[test]
    fn a_failed_refresh_keeps_the_old_value() {
        let mut slot = Slot::default();

        slot.apply(Load::Ready { value: 1 });
        slot.apply(Load::Failed(LibraryError::NotFound));

        assert_eq!(slot.value(), Some(&1));
        assert!(slot.error().is_none());

        let mut empty: Slot<u8> = Slot::default();

        empty.apply(Load::Failed(LibraryError::NotFound));

        assert!(empty.error().is_some());
    }
}
