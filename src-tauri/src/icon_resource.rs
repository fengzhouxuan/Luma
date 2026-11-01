//! 按快捷方式指定的资源索引提取 96px 图标，支持正序号和负资源 ID。

use base64::Engine;
use std::os::windows::ffi::OsStrExt;
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits,
    GetObjectW, HBITMAP, HDC, ReleaseDC,
};
use windows_sys::Win32::UI::Shell::SHDefExtractIconW;
use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};

struct Icon(HICON);
impl Drop for Icon {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                DestroyIcon(self.0);
            }
        }
    }
}

struct Bitmap(HBITMAP);
impl Drop for Bitmap {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                DeleteObject(self.0);
            }
        }
    }
}

struct Dc(HDC);
impl Drop for Dc {
    fn drop(&mut self) {
        unsafe {
            ReleaseDC(std::ptr::null_mut(), self.0);
        }
    }
}

fn read_bitmap(bitmap: HBITMAP) -> Result<(u32, u32, Vec<u8>), String> {
    unsafe {
        let mut details: BITMAP = std::mem::zeroed();
        if GetObjectW(
            bitmap,
            std::mem::size_of::<BITMAP>() as i32,
            &mut details as *mut _ as _,
        ) == 0
        {
            return Err("读取图标位图信息失败".into());
        }
        let width = details.bmWidth.unsigned_abs();
        let height = details.bmHeight.unsigned_abs();
        if width == 0 || height == 0 || width > 1024 || height > 2048 {
            return Err("图标位图尺寸无效".into());
        }
        let dc = Dc(GetDC(std::ptr::null_mut()));
        if dc.0.is_null() {
            return Err("创建图标 DC 失败".into());
        }
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            biHeight: -(height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        if GetDIBits(
            dc.0,
            bitmap,
            0,
            height,
            pixels.as_mut_ptr() as _,
            &mut info,
            DIB_RGB_COLORS,
        ) != height as i32
        {
            return Err("读取图标像素失败".into());
        }
        Ok((width, height, pixels))
    }
}

pub fn extract(path: &str, index: i32) -> Result<String, String> {
    let wide: Vec<u16> = std::ffi::OsStr::new(path)
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut icon = Icon(std::ptr::null_mut());
    let hr = unsafe {
        SHDefExtractIconW(
            wide.as_ptr(),
            index,
            0,
            &mut icon.0,
            std::ptr::null_mut(),
            96,
        )
    };
    if hr != 0 || icon.0.is_null() {
        return Err(format!(
            "读取图标资源失败 ({path}, index={index}, HRESULT={hr:#x})"
        ));
    }
    let mut info: ICONINFO = unsafe { std::mem::zeroed() };
    if unsafe { GetIconInfo(icon.0, &mut info) } == 0 {
        return Err("读取 HICON 失败".into());
    }
    let color = Bitmap(info.hbmColor);
    let mask = Bitmap(info.hbmMask);
    let (width, height, mut pixels) = if !color.0.is_null() {
        read_bitmap(color.0)?
    } else {
        let (width, full_height, all) = read_bitmap(mask.0)?;
        if full_height % 2 != 0 {
            return Err("单色图标掩码高度无效".into());
        }
        let offset = width as usize * (full_height as usize / 2) * 4;
        (width, full_height / 2, all[offset..].to_vec())
    };
    // 旧图标没有 alpha，使用 AND 掩码恢复透明度。
    if pixels.chunks_exact(4).all(|pixel| pixel[3] == 0) {
        let (_, _, mask_pixels) = read_bitmap(mask.0)?;
        if mask_pixels.len() < pixels.len() {
            return Err("图标掩码尺寸无效".into());
        }
        for (pixel, mask) in pixels.chunks_exact_mut(4).zip(mask_pixels.chunks_exact(4)) {
            pixel[3] = if mask[0] == 0 { 255 } else { 0 };
        }
    }
    encode_pixels(width, height, pixels)
}

pub fn shell_bitmap(bitmap: HBITMAP) -> Result<String, String> {
    let bitmap = Bitmap(bitmap);
    let (width, height, mut pixels) = read_bitmap(bitmap.0)?;
    if pixels.chunks_exact(4).all(|pixel| pixel[3] == 0) {
        for pixel in pixels.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
    }
    encode_pixels(width, height, pixels)
}

fn encode_pixels(width: u32, height: u32, mut pixels: Vec<u8>) -> Result<String, String> {
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        let alpha = pixel[3] as u32;
        if alpha > 0 && alpha < 255 {
            for value in &mut pixel[..3] {
                *value = ((*value as u32 * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    let image = image::RgbaImage::from_raw(width, height, pixels).ok_or("图标像素长度无效")?;
    let mut png = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png.into_inner())
    ))
}
