use ico::{IconDir, IconDirEntry, IconImage, ResourceType};
use std::fs::File;
use std::path::PathBuf;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let output = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR is set"));
    let icon_path = output.join("agent-chat-cleaner.ico");
    write_icon(&icon_path);

    let manifest = r#"
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security><requestedPrivileges><requestedExecutionLevel level="asInvoker" uiAccess="false"/></requestedPrivileges></security>
  </trustInfo>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
      <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
    </windowsSettings>
  </application>
</assembly>
"#;

    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon(icon_path.to_string_lossy().as_ref())
        .set_manifest(manifest)
        .compile()
        .expect("compile Windows icon and metadata");
}

fn write_icon(path: &std::path::Path) {
    let mut directory = IconDir::new(ResourceType::Icon);
    for size in [16_u32, 24, 32, 48, 64, 128, 256] {
        let image = IconImage::from_rgba_data(size, size, render_icon(size));
        directory.add_entry(IconDirEntry::encode(&image).expect("encode icon frame"));
    }
    directory
        .write(File::create(path).expect("create icon"))
        .expect("write icon");
}

fn render_icon(size: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let mut sum = [0_u32; 4];
            for sample_y in 0..4 {
                for sample_x in 0..4 {
                    let px = (x as f32 + (sample_x as f32 + 0.5) / 4.0) / size as f32;
                    let py = (y as f32 + (sample_y as f32 + 0.5) / 4.0) / size as f32;
                    let color = sample_logo(px, py);
                    for channel in 0..4 {
                        sum[channel] += u32::from(color[channel]);
                    }
                }
            }
            pixels.extend(sum.map(|value| (value / 16) as u8));
        }
    }
    pixels
}

fn sample_logo(x: f32, y: f32) -> [u8; 4] {
    let radius = 0.21;
    let dx = (0.12 - x).max(0.0).max(x - 0.88);
    let dy = (0.12 - y).max(0.0).max(y - 0.88);
    if dx * dx + dy * dy > radius * radius {
        return [0, 0, 0, 0];
    }

    let background = [15, 58, 49, 255];
    let bars = [
        (0.27_f32, 0.53_f32, 0.79_f32),
        (0.43_f32, 0.39_f32, 0.79_f32),
        (0.59_f32, 0.25_f32, 0.79_f32),
    ];
    for (left, top, bottom) in bars {
        let shear = (bottom - y) * 0.18;
        let local_x = x - shear;
        if y >= top && y <= bottom && local_x >= left && local_x <= left + 0.105 {
            return [210, 242, 109, 255];
        }
    }
    background
}
