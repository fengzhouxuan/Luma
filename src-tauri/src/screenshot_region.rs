use crate::position::Rect;
use image::RgbaImage;
use std::{io, mem, ptr, slice};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GdiFlush, GetDC, HBITMAP, HDC, HGDIOBJ, ReleaseDC,
    SRCCOPY, SelectObject,
};

/// One reusable bitmap for the selected region. All GDI operations, including
/// dropping the sampler, run on the capture thread that acquired the desktop DC.
pub(crate) struct RegionSampler {
    area: Rect,
    desktop: HDC,
    memory: HDC,
    bitmap: HBITMAP,
    original: HGDIOBJ,
    pixels: *mut u8,
}

impl RegionSampler {
    pub(crate) fn new(area: Rect) -> Result<Self, String> {
        let mut sampler = Self {
            area,
            desktop: ptr::null_mut(),
            memory: ptr::null_mut(),
            bitmap: ptr::null_mut(),
            original: ptr::null_mut(),
            pixels: ptr::null_mut(),
        };
        unsafe {
            sampler.desktop = GetDC(ptr::null_mut());
            if sampler.desktop.is_null() {
                return Err(gdi_error("打开桌面设备上下文"));
            }
            sampler.memory = CreateCompatibleDC(sampler.desktop);
            if sampler.memory.is_null() {
                return Err(gdi_error("创建设备上下文"));
            }
            let mut info: BITMAPINFO = mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: area.width as i32,
                biHeight: -(area.height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..mem::zeroed()
            };
            let mut bits = ptr::null_mut();
            sampler.bitmap = CreateDIBSection(
                sampler.desktop,
                &info,
                DIB_RGB_COLORS,
                &mut bits,
                ptr::null_mut(),
                0,
            );
            if sampler.bitmap.is_null() || bits.is_null() {
                return Err(gdi_error("创建选区位图"));
            }
            let original = SelectObject(sampler.memory, sampler.bitmap);
            if original.is_null() || original as isize == -1 {
                return Err(gdi_error("选入采集位图"));
            }
            sampler.original = original;
            sampler.pixels = bits.cast();
        }
        Ok(sampler)
    }

    pub(crate) fn sample(&mut self) -> Result<RgbaImage, String> {
        let area = self.area;
        let pixels = unsafe {
            if BitBlt(
                self.memory,
                0,
                0,
                area.width as i32,
                area.height as i32,
                self.desktop,
                area.x,
                area.y,
                SRCCOPY,
            ) == 0
            {
                return Err(gdi_error("读取截图选区"));
            }
            // Flush this thread's GDI batch before reading the DIB directly.
            if GdiFlush() == 0 {
                return Err(gdi_error("同步采集位图"));
            }
            slice::from_raw_parts(self.pixels, area.width as usize * area.height as usize * 4)
        };
        let mut rgba = pixels.to_vec();
        for pixel in rgba.chunks_exact_mut(4) {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
        RgbaImage::from_raw(area.width, area.height, rgba).ok_or_else(|| "截图尺寸无效".into())
    }
}

impl Drop for RegionSampler {
    fn drop(&mut self) {
        unsafe {
            if !self.original.is_null() {
                SelectObject(self.memory, self.original);
            }
            if !self.memory.is_null() {
                DeleteDC(self.memory);
            }
            if !self.bitmap.is_null() {
                DeleteObject(self.bitmap);
            }
            if !self.desktop.is_null() {
                ReleaseDC(ptr::null_mut(), self.desktop);
            }
        }
    }
}

fn gdi_error(operation: &str) -> String {
    format!("{operation}失败：{}", io::Error::last_os_error())
}
