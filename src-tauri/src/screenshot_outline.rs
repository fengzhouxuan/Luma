use crate::position::Rect;
use std::{io, mem, ptr, sync::OnceLock};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{
        BLACK_BRUSH, BeginPaint, DC_BRUSH, EndPaint, FillRect, FrameRect, GetStockObject,
        InflateRect, PAINTSTRUCT, SetDCBrushColor,
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetWindowDisplayAffinity,
        LWA_COLORKEY, RegisterClassW, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetLayeredWindowAttributes,
        SetWindowDisplayAffinity, SetWindowPos, WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WM_PAINT,
        WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
        WS_EX_TRANSPARENT, WS_POPUP,
    },
};

static CLASS: OnceLock<Result<u16, String>> = OnceLock::new();
const TRANSPARENT_COLOR: u32 = 0;
const BORDER_COLORS: [u32; 3] = [0x0028_1b_14, 0x00ff_e385, 0x00ff_e385];

fn outline_error() -> String {
    format!("无法显示长截图选框：{}", io::Error::last_os_error())
}

pub(crate) struct Outline(isize);

impl Outline {
    // Create and destroy on the UI thread. The handle travels with the capture's completion callback.
    pub(crate) fn show(area: Rect, control: HWND) -> Result<Self, String> {
        unsafe {
            let instance = GetModuleHandleW(ptr::null());
            let atom = CLASS.get_or_init(|| {
                let name = windows::core::w!("LumaLongScreenshotOutline");
                let class = WNDCLASSW {
                    lpfnWndProc: Some(paint_outline),
                    hInstance: instance,
                    lpszClassName: name.as_ptr(),
                    ..mem::zeroed()
                };
                match RegisterClassW(&class) {
                    0 => Err(outline_error()),
                    atom => Ok(atom),
                }
            });
            let atom = *atom.as_ref().map_err(Clone::clone)?;
            // The layered window passes mouse input through to the scrolling app.
            let window = CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST,
                atom as usize as *const u16,
                ptr::null(),
                WS_POPUP,
                area.x,
                area.y,
                area.width as i32,
                area.height as i32,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                ptr::null(),
            );
            if window.is_null() {
                return Err(outline_error());
            }
            let outline = Self(window as isize);
            if SetLayeredWindowAttributes(window, TRANSPARENT_COLOR, 0, LWA_COLORKEY) == 0 {
                let error = outline_error();
                outline.close();
                return Err(error);
            }
            let mut affinity = WDA_NONE;
            let excluded = SetWindowDisplayAffinity(window, WDA_EXCLUDEFROMCAPTURE) != 0
                && GetWindowDisplayAffinity(window, &mut affinity) != 0
                && affinity == WDA_EXCLUDEFROMCAPTURE;
            if !excluded {
                SetWindowDisplayAffinity(window, WDA_NONE);
            }
            // On older Windows, place the painted pixels outside the sampled rectangle.
            let outset = if excluded {
                0
            } else {
                BORDER_COLORS.len() as i32
            };
            if SetWindowPos(
                window,
                control,
                area.x - outset,
                area.y - outset,
                area.width as i32 + 2 * outset,
                area.height as i32 + 2 * outset,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            ) == 0
            {
                let error = outline_error();
                outline.close();
                return Err(error);
            }
            Ok(outline)
        }
    }

    pub(crate) fn close(self) {
        unsafe { DestroyWindow(self.0 as HWND) };
    }
}

unsafe extern "system" fn paint_outline(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if message != WM_PAINT {
            return DefWindowProcW(window, message, wparam, lparam);
        }
        let mut paint: PAINTSTRUCT = mem::zeroed();
        let dc = BeginPaint(window, &mut paint);
        let mut rect: RECT = mem::zeroed();
        GetClientRect(window, &mut rect);
        FillRect(dc, &rect, GetStockObject(BLACK_BRUSH));
        let brush = GetStockObject(DC_BRUSH);
        for color in BORDER_COLORS {
            SetDCBrushColor(dc, color);
            FrameRect(dc, &rect, brush);
            InflateRect(&mut rect, -1, -1);
        }
        EndPaint(window, &paint);
        0
    }
}
