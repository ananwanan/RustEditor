//! System fonts are validated before installation and never redistributed.
use ab_glyph::{Font, FontRef};
use eframe::egui::{self, FontFamily};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_FONT_BYTES: u64 = 64 * 1024 * 1024;

pub fn install(ctx: &egui::Context) -> String {
    let mut fonts = egui::FontDefinitions::default();
    let mut status = Vec::new();
    for (name, variable, required, defaults) in [
        ("system-cjk", "RUSTEDITOR_FONT", "你好", cjk_paths()),
        (
            "system-emoji",
            "RUSTEDITOR_EMOJI_FONT",
            "🙂👩💻",
            emoji_paths(),
        ),
    ] {
        let explicit = std::env::var_os(variable).map(PathBuf::from);
        let mut loaded = false;
        for path in explicit.iter().cloned().chain(defaults) {
            if !path.exists() && explicit.as_ref() != Some(&path) {
                continue;
            }
            match read_font(&path, required) {
                Ok(bytes) => {
                    fonts
                        .font_data
                        .insert(name.into(), egui::FontData::from_owned(bytes).into());
                    for family in [FontFamily::Proportional, FontFamily::Monospace] {
                        let fallbacks = fonts.families.entry(family).or_default();
                        // Keep the primary Latin/code font; prefer the installed
                        // emoji font over the bundled, deliberately small subset.
                        if name == "system-emoji" {
                            let index = fallbacks
                                .iter()
                                .position(|n| n.contains("Emoji") || n.contains("emoji"))
                                .unwrap_or(fallbacks.len());
                            fallbacks.insert(index, name.into());
                        } else {
                            fallbacks.push(name.into());
                        }
                    }
                    status.push(
                        if name == "system-cjk" {
                            "CJK font loaded"
                        } else {
                            "Emoji outline font loaded"
                        }
                        .into(),
                    );
                    loaded = true;
                    break;
                }
                Err(error) => status.push(format!("{}: {error}", path.display())),
            }
        }
        if !loaded {
            status.push(format!(
                "{name} unavailable; set {variable} to an outline TTF/TTC/OTF font"
            ));
        }
    }
    ctx.set_fonts(fonts);
    status.push(
        "Emoji render as monochrome glyphs; color/ZWJ ligature shaping is not supported.".into(),
    );
    status.join("\n")
}

fn validate_bytes(bytes: &[u8], required: &str) -> Result<(), String> {
    let font =
        FontRef::try_from_slice(bytes).map_err(|_| "invalid or unsupported font".to_owned())?;
    for character in required.chars() {
        let glyph = font.glyph_id(character);
        if glyph.0 == 0 || font.outline_glyph(glyph.with_scale(16.0)).is_none() {
            return Err(format!("no outline glyph for U+{:04X}", character as u32));
        }
    }
    Ok(())
}

fn read_font(path: &Path, required: &str) -> Result<Vec<u8>, String> {
    let metadata = path.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_FONT_BYTES {
        return Err("font must be a regular file of at most 64 MiB".into());
    }
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_FONT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_FONT_BYTES {
        return Err("font exceeds 64 MiB".into());
    }
    validate_bytes(&bytes, required)?;
    Ok(bytes)
}

fn cjk_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(windows) = std::env::var_os("WINDIR") {
        paths.push(PathBuf::from(windows).join("Fonts/msyh.ttc"));
    }
    paths.extend([
        PathBuf::from("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"),
        PathBuf::from("/System/Library/Fonts/PingFang.ttc"),
    ]);
    paths
}

fn emoji_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(windows) = std::env::var_os("WINDIR") {
        paths.push(PathBuf::from(windows).join("Fonts/seguiemj.ttf"));
    }
    paths.extend([
        PathBuf::from("/usr/share/fonts/truetype/noto/NotoEmoji-Regular.ttf"),
        PathBuf::from("/usr/share/fonts/truetype/noto/NotoEmoji[wght].ttf"),
        PathBuf::from("/System/Library/Fonts/Apple Color Emoji.ttc"),
    ]);
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_and_missing_glyph_fonts_are_rejected_before_registration() {
        assert!(validate_bytes(b"not a font", "A").is_err());
        let fonts = egui::FontDefinitions::default();
        let latin = &fonts.font_data["Hack"].font;
        assert!(validate_bytes(latin, "AZ").is_ok());
        assert!(
            validate_bytes(latin, "你好")
                .unwrap_err()
                .contains("U+4F60")
        );
    }
}
