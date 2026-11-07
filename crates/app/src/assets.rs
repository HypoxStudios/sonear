use gpui_kit::assets::{Assets, icon_assets};
use gpui_kit::{AssetSource, SharedString};

icon_assets!(
    AppIcons,
    [
        House,
        Search,
        Library,
        Heart,
        Shuffle,
        SkipBack,
        SkipForward,
        Play,
        Pause,
        Repeat,
        Repeat1,
        Volume1,
        Volume2,
        VolumeX,
        Sparkles,
        ListMusic,
        ListPlus,
        GripVertical,
        MicVocal,
        Music,
        PanelLeftClose,
        PanelLeftOpen,
        X,
        ChevronLeft,
        ChevronRight,
        ExternalLink,
        KeyRound,
        Download,
        TriangleAlert,
        User,
        LoaderCircle,
        RefreshCw,
        CircleCheck,
        Clock
    ]
);

pub const MARK: &str = "sonear/mark.svg";
pub const MARK_SMALL: &str = "sonear/mark-small.svg";
pub const PLAY_FILLED: &str = "sonear/play.svg";
pub const PAUSE_FILLED: &str = "sonear/pause.svg";
pub const HEART_FILLED: &str = "sonear/heart.svg";

const OWN_ASSETS: [(&str, &[u8]); 5] = [
    (MARK, include_bytes!("../assets/mark.svg")),
    (MARK_SMALL, include_bytes!("../assets/mark-small.svg")),
    (PLAY_FILLED, include_bytes!("../assets/play.svg")),
    (PAUSE_FILLED, include_bytes!("../assets/pause.svg")),
    (HEART_FILLED, include_bytes!("../assets/heart.svg")),
];

/// The default component icons plus the extra Lucide icons and artwork Sonear draws.
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = OWN_ASSETS.iter().find(|(own, _)| *own == path) {
            return Ok(Some(std::borrow::Cow::Borrowed(*bytes)));
        }

        if let Some(bytes) = AppIcons.load(path)? {
            return Ok(Some(bytes));
        }

        Assets.load(path)
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut paths = Assets.list(path)?;

        paths.extend(AppIcons.list(path)?);
        paths.sort();
        paths.dedup();

        Ok(paths)
    }
}
