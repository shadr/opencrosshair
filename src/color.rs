/// Parse hex color string to RGBA float array
/// Supports formats: RGB, #RGB, RRGGBB, #RRGGBB, RRGGBBAA, #RRGGBBAA
pub fn parse_hex_color(color: &str) -> Result<[f32; 4], String> {
    let color = color.trim();

    // Remove optional '#' prefix
    let hex = color.strip_prefix('#').unwrap_or(color);
    let len = hex.len();

    let (r, g, b, a) = match len {
        3 => {
            // #RGB format
            let chars: Vec<char> = hex.chars().collect();
            let r = u8::from_str_radix(&format!("{}{}", chars[0], chars[0]), 16)
                .map_err(|_| "Invalid red component")?;
            let g = u8::from_str_radix(&format!("{}{}", chars[1], chars[1]), 16)
                .map_err(|_| "Invalid green component")?;
            let b = u8::from_str_radix(&format!("{}{}", chars[2], chars[2]), 16)
                .map_err(|_| "Invalid blue component")?;
            let a = 255u8;
            (r, g, b, a)
        }
        6 => {
            // #RRGGBB format
            let r = u8::from_str_radix(&hex[0..2], 16)
                .map_err(|_| "Invalid red component")?;
            let g = u8::from_str_radix(&hex[2..4], 16)
                .map_err(|_| "Invalid green component")?;
            let b = u8::from_str_radix(&hex[4..6], 16)
                .map_err(|_| "Invalid blue component")?;
            let a = 255u8;
            (r, g, b, a)
        }
        8 => {
            // #RRGGBBAA format
            let r = u8::from_str_radix(&hex[0..2], 16)
                .map_err(|_| "Invalid red component")?;
            let g = u8::from_str_radix(&hex[2..4], 16)
                .map_err(|_| "Invalid green component")?;
            let b = u8::from_str_radix(&hex[4..6], 16)
                .map_err(|_| "Invalid blue component")?;
            let a = u8::from_str_radix(&hex[6..8], 16)
                .map_err(|_| "Invalid alpha component")?;
            (r, g, b, a)
        }
        _ => {
            return Err("Color must be in format RGB, RRGGBB, or RRGGBBAA".to_string());
        }
    };

    Ok([
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    ])
}
