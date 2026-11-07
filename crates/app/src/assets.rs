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
        Clock,
        Settings,
        PictureInPicture2,
        Maximize2
    ]
);

pub const MARK: &str = "sonear/mark.svg";
pub const MARK_SMALL: &str = "sonear/mark-small.svg";
pub const PLAY_FILLED: &str = "sonear/play.svg";
pub const PAUSE_FILLED: &str = "sonear/pause.svg";
pub const HEART_FILLED: &str = "sonear/heart.svg";

pub const SPOTIFY_LOGO: &str = "sonear/spotify-logo.svg";
pub const SPOTIFY_ICON: &str = "sonear/spotify-icon.svg";

const OWN_ASSETS: [(&str, &[u8]); 7] = [
    (SPOTIFY_LOGO, include_bytes!("../assets/spotify-logo.svg")),
    (SPOTIFY_ICON, include_bytes!("../assets/spotify-icon.svg")),
    (MARK, include_bytes!("../assets/mark.svg")),
    (MARK_SMALL, include_bytes!("../assets/mark-small.svg")),
    (PLAY_FILLED, include_bytes!("../assets/play.svg")),
    (PAUSE_FILLED, include_bytes!("../assets/pause.svg")),
    (HEART_FILLED, include_bytes!("../assets/heart.svg")),
];

#[cfg(feature = "demo")]
macro_rules! demo_covers {
    ($($n:literal),*) => {
        [$((
            concat!("demo/cover-", $n, ".jpg"),
            include_bytes!(concat!("../assets/demo/cover-", $n, ".jpg")).as_slice(),
        )),*]
    };
}

#[cfg(feature = "demo")]
pub const DEMO_COVERS: [(&str, &[u8]); 16] = demo_covers!(
    "00", "01", "02", "03", "04", "05", "06", "07", "08", "09", "10", "11", "12", "13", "14", "15"
);

/// The default component icons plus the extra Lucide icons and artwork Sonear draws.
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = OWN_ASSETS.iter().find(|(own, _)| *own == path) {
            return Ok(Some(std::borrow::Cow::Borrowed(*bytes)));
        }

        #[cfg(feature = "demo")]
        if let Some((_, bytes)) = DEMO_COVERS.iter().find(|(own, _)| *own == path) {
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
