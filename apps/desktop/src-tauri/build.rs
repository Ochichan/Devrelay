fn main() {
    ensure_icon();
    #[cfg(target_os = "macos")]
    ensure_macos_frameworks();
    tauri_build::build();
}

fn ensure_icon() {
    std::fs::create_dir_all("icons").expect("failed to create Tauri icon directory");
    let png = render_icon_png(1024, 1024);
    std::fs::write(std::path::Path::new("icons").join("icon.png"), &png)
        .expect("failed to write generated Tauri icon");
    std::fs::write(
        std::path::Path::new("icons").join("icon.icns"),
        render_icon_icns(),
    )
    .expect("failed to write generated macOS icon set");
}

#[cfg(target_os = "macos")]
fn ensure_macos_frameworks() {
    let framework_dir = std::path::Path::new("frameworks");
    std::fs::create_dir_all(framework_dir).expect("failed to create Tauri framework directory");

    let source = find_macos_libiconv().unwrap_or_else(|| {
        panic!(
            "failed to locate libiconv.2.dylib; set DEVRELAY_LIBICONV_DYLIB to an absolute dylib path"
        )
    });
    let target = framework_dir.join("libiconv.2.dylib");
    std::fs::copy(&source, &target).unwrap_or_else(|error| {
        panic!(
            "failed to copy {} to {}: {error}",
            source.display(),
            target.display()
        )
    });

    let mut permissions = std::fs::metadata(&target)
        .expect("failed to read generated libiconv permissions")
        .permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o644);
    }
    #[cfg(not(unix))]
    {
        permissions.set_readonly(false);
    }
    std::fs::set_permissions(&target, permissions)
        .expect("failed to make generated libiconv writable");
}

#[cfg(target_os = "macos")]
fn find_macos_libiconv() -> Option<std::path::PathBuf> {
    std::env::var_os("DEVRELAY_LIBICONV_DYLIB")
        .map(std::path::PathBuf::from)
        .filter(|path| path.exists())
        .or_else(|| {
            [
                "/opt/homebrew/opt/libiconv/lib/libiconv.2.dylib",
                "/usr/local/opt/libiconv/lib/libiconv.2.dylib",
            ]
            .into_iter()
            .map(std::path::PathBuf::from)
            .find(|path| path.exists())
        })
}

fn render_icon_png(width: u32, height: u32) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((height * (1 + width * 4)) as usize);
    for y in 0..height {
        rgba.push(0);
        for x in 0..width {
            let color = icon_pixel(x as f32 + 0.5, y as f32 + 0.5, width as f32, height as f32);
            rgba.extend_from_slice(&color);
        }
    }

    let mut png = Vec::new();
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    push_chunk(&mut png, b"IHDR", &ihdr);
    push_chunk(&mut png, b"IDAT", &zlib_store(&rgba));
    push_chunk(&mut png, b"IEND", &[]);
    png
}

fn render_icon_icns() -> Vec<u8> {
    let entries = [
        (*b"icp4", render_icon_png(16, 16)),
        (*b"icp5", render_icon_png(32, 32)),
        (*b"icp6", render_icon_png(64, 64)),
        (*b"ic07", render_icon_png(128, 128)),
        (*b"ic08", render_icon_png(256, 256)),
        (*b"ic09", render_icon_png(512, 512)),
        (*b"ic10", render_icon_png(1024, 1024)),
    ];
    let total_len = 8 + entries.iter().map(|(_, png)| 8 + png.len()).sum::<usize>();
    let mut icns = Vec::with_capacity(total_len);
    icns.extend_from_slice(b"icns");
    icns.extend_from_slice(&(total_len as u32).to_be_bytes());
    for (kind, png) in entries {
        icns.extend_from_slice(&kind);
        icns.extend_from_slice(&((8 + png.len()) as u32).to_be_bytes());
        icns.extend_from_slice(&png);
    }
    icns
}

fn icon_pixel(px: f32, py: f32, width: f32, height: f32) -> [u8; 4] {
    let x = px / width;
    let y = py / height;
    let d = rounded_rect_sdf(x, y, 0.5, 0.5, 0.45, 0.45, 0.19);
    let mask = smoothstep(0.018, -0.006, d);
    if mask <= 0.0 {
        return [0, 0, 0, 0];
    }

    let mut r = mix(12.0, 30.0, y);
    let mut g = mix(17.0, 39.0, y);
    let mut b = mix(25.0, 53.0, y);

    let top_glow = radial(x, y, 0.27, 0.18, 0.56);
    r += top_glow * 13.0;
    g += top_glow * 34.0;
    b += top_glow * 44.0;

    let relay_glow = relay_sdf(x, y, 0.084);
    let glow = smoothstep(0.19, 0.0, relay_glow);
    r += glow * 15.0;
    g += glow * 61.0;
    b += glow * 54.0;

    let mark = smoothstep(0.013, -0.006, relay_sdf(x, y, 0.0));
    let mark_core = smoothstep(0.0, -0.014, relay_sdf(x, y, 0.0));
    r = mix(r, 82.0 + mark_core * 40.0, mark);
    g = mix(g, 233.0 + mark_core * 14.0, mark);
    b = mix(b, 188.0 + mark_core * 32.0, mark);

    let node = node_mask(x, y);
    r = mix(r, 233.0, node);
    g = mix(g, 255.0, node);
    b = mix(b, 246.0, node);

    let grid = grid_mask(x, y) * (1.0 - mark * 0.8);
    r += grid * 12.0;
    g += grid * 15.0;
    b += grid * 18.0;

    let edge = smoothstep(0.012, -0.004, d) - smoothstep(-0.03, -0.06, d);
    r += edge * 22.0;
    g += edge * 25.0;
    b += edge * 28.0;

    let vignette = radial(x, y, 0.5, 0.5, 0.82);
    r *= mix(0.78, 1.0, vignette);
    g *= mix(0.78, 1.0, vignette);
    b *= mix(0.82, 1.0, vignette);

    [
        (r.clamp(0.0, 255.0) * mask) as u8,
        (g.clamp(0.0, 255.0) * mask) as u8,
        (b.clamp(0.0, 255.0) * mask) as u8,
        (255.0 * mask) as u8,
    ]
}

fn relay_sdf(x: f32, y: f32, expand: f32) -> f32 {
    let stroke = 0.045 + expand;
    let mut d: f32 = 1.0;
    d = d.min(dist_to_segment(x, y, 0.25, 0.39, 0.67, 0.39) - stroke);
    d = d.min(dist_to_segment(x, y, 0.33, 0.61, 0.75, 0.61) - stroke);
    d = d.min(dist_to_segment(x, y, 0.25, 0.39, 0.38, 0.27) - stroke);
    d = d.min(dist_to_segment(x, y, 0.25, 0.39, 0.38, 0.51) - stroke);
    d = d.min(dist_to_segment(x, y, 0.75, 0.61, 0.62, 0.49) - stroke);
    d = d.min(dist_to_segment(x, y, 0.75, 0.61, 0.62, 0.73) - stroke);
    d = d.min(dist_to_segment(x, y, 0.44, 0.50, 0.56, 0.50) - (0.038 + expand));
    d
}

fn node_mask(x: f32, y: f32) -> f32 {
    let left = smoothstep(
        0.052,
        0.026,
        ((x - 0.25).powi(2) + (y - 0.39).powi(2)).sqrt(),
    );
    let right = smoothstep(
        0.052,
        0.026,
        ((x - 0.75).powi(2) + (y - 0.61).powi(2)).sqrt(),
    );
    (left + right).clamp(0.0, 1.0)
}

fn grid_mask(x: f32, y: f32) -> f32 {
    let gx = ((x * 7.0).fract() - 0.5).abs();
    let gy = ((y * 7.0).fract() - 0.5).abs();
    let line = smoothstep(0.012, 0.0, gx.min(gy));
    let fade = smoothstep(0.08, 0.24, x) * smoothstep(0.92, 0.70, x);
    line * fade * 0.18
}

fn rounded_rect_sdf(x: f32, y: f32, cx: f32, cy: f32, hx: f32, hy: f32, radius: f32) -> f32 {
    let qx = (x - cx).abs() - hx + radius;
    let qy = (y - cy).abs() - hy + radius;
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius
}

fn radial(x: f32, y: f32, cx: f32, cy: f32, radius: f32) -> f32 {
    (1.0 - (((x - cx).powi(2) + (y - cy).powi(2)).sqrt() / radius)).clamp(0.0, 1.0)
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

fn dist_to_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let vx = bx - ax;
    let vy = by - ay;
    let wx = px - ax;
    let wy = py - ay;
    let c = ((wx * vx + wy * vy) / (vx * vx + vy * vy)).clamp(0.0, 1.0);
    let dx = px - (ax + c * vx);
    let dy = py - (ay + c * vy);
    dx.hypot(dy)
}

fn zlib_store(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    for (i, chunk) in data.chunks(65_535).enumerate() {
        out.push(if (i + 1) * 65_535 >= data.len() {
            0x01
        } else {
            0x00
        });
        let len = chunk.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(chunk);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn push_chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(kind.len() + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    png.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                0xedb8_8320 ^ (crc >> 1)
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let mut a = 1_u32;
    let mut b = 0_u32;
    for &byte in data {
        a = (a + byte as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}
