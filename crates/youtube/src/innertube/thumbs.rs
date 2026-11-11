use crate::models::Thumb;

const SIZES: [u32; 3] = [120, 300, 544];

/// Rewrites `=w60-h60-l90-rj` style size suffixes. Only square sizes on
/// Google's image hosts can be rewritten.
fn resize(url: &str, size: u32) -> Option<String> {
    if !(url.contains("googleusercontent.com") || url.contains("ggpht.com")) {
        return None;
    }

    let start = url.rfind("=w")?;
    let rest = &url[start + 2..];
    let width_len = rest.bytes().take_while(u8::is_ascii_digit).count();
    let after_width = rest[width_len..].strip_prefix("-h")?;
    let height_len = after_width.bytes().take_while(u8::is_ascii_digit).count();

    if width_len == 0 || height_len == 0 || rest[..width_len] != after_width[..height_len] {
        return None;
    }

    let tail = &after_width[height_len..];

    if !(tail.is_empty() || tail.starts_with('-')) {
        return None;
    }

    Some(format!("{}=w{size}-h{size}{tail}", &url[..start]))
}

/// A few sizes (120, 300 and 544 px) synthesized from the first resizable
/// URL, or the list as it came when none can be resized.
pub fn variants(thumbs: &[Thumb]) -> Vec<Thumb> {
    for thumb in thumbs {
        let sized: Vec<Thumb> = SIZES
            .iter()
            .filter_map(|&size| {
                Some(Thumb {
                    url: resize(&thumb.url, size)?,
                    width: Some(size),
                })
            })
            .collect();

        if sized.len() == SIZES.len() {
            return sized;
        }
    }

    thumbs.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thumb(url: &str) -> Thumb {
        Thumb {
            url: url.to_owned(),
            width: Some(60),
        }
    }

    #[test]
    fn google_images_get_three_sizes() {
        let sized = variants(&[thumb(
            "https://lh3.googleusercontent.com/abc=w60-h60-l90-rj",
        )]);

        assert_eq!(sized.len(), 3);
        assert_eq!(
            sized[2].url,
            "https://lh3.googleusercontent.com/abc=w544-h544-l90-rj"
        );
        assert_eq!(sized[0].width, Some(120));
    }

    #[test]
    fn the_crop_flag_is_kept() {
        let sized = variants(&[thumb(
            "https://yt3.googleusercontent.com/abc=w60-h60-p-l90-rj",
        )]);

        assert_eq!(
            sized[1].url,
            "https://yt3.googleusercontent.com/abc=w300-h300-p-l90-rj"
        );
    }

    #[test]
    fn other_urls_come_back_untouched() {
        let raw = vec![
            thumb("https://i.ytimg.com/vi/x/hqdefault.jpg?sqp=abc"),
            thumb("https://lh3.googleusercontent.com/abc=w60-h90-l90-rj"),
        ];

        assert_eq!(variants(&raw), raw);
    }
}
