//! Minimal FFI to the unmodified official SDK sources in vendor/.
//! All SDK calls happen in the disposable worker, never in the UI process.
use super::{FileEntry, Request, SearchResult};
use std::ffi::c_void;

type Handle = *mut c_void;

unsafe extern "system" {
    fn Everything3_ConnectW(instance: *const u16) -> Handle;
    fn Everything3_DestroyClient(client: Handle) -> i32;
    fn Everything3_GetMajorVersion(client: Handle) -> u32;
    fn Everything3_GetMinorVersion(client: Handle) -> u32;
    fn Everything3_GetRevision(client: Handle) -> u32;
    fn Everything3_GetBuildNumber(client: Handle) -> u32;
    fn Everything3_IsDBLoaded(client: Handle) -> i32;
    fn Everything3_CreateSearchState() -> Handle;
    fn Everything3_DestroySearchState(state: Handle) -> i32;
    fn Everything3_SetSearchTextW(state: Handle, query: *const u16) -> i32;
    fn Everything3_SetSearchViewportOffset(state: Handle, offset: usize) -> i32;
    fn Everything3_SetSearchViewportCount(state: Handle, count: usize) -> i32;
    fn Everything3_GetSearchViewportOffset(state: Handle) -> usize;
    fn Everything3_GetSearchViewportCount(state: Handle) -> usize;
    fn Everything3_AddSearchPropertyRequest(state: Handle, property: u32) -> i32;
    fn Everything3_AddSearchSort(state: Handle, property: u32, ascending: i32) -> i32;
    fn Everything3_Search(client: Handle, state: Handle) -> Handle;
    fn Everything3_DestroyResultList(list: Handle) -> i32;
    fn Everything3_GetResultListCount(list: Handle) -> usize;
    fn Everything3_GetResultListViewportCount(list: Handle) -> usize;
    fn Everything3_GetResultFullPathNameW(
        list: Handle,
        index: usize,
        buffer: *mut u16,
        length: usize,
    ) -> usize;
    fn Everything3_IsFolderResult(list: Handle, index: usize) -> i32;
    fn Everything3_GetResultSize(list: Handle, index: usize) -> u64;
    fn Everything3_GetLastError() -> u32;

    fn Everything_GetMajorVersion() -> u32;
    fn Everything_GetMinorVersion() -> u32;
    fn Everything_GetRevision() -> u32;
    fn Everything_GetBuildNumber() -> u32;
    fn Everything_IsDBLoaded() -> i32;
    fn Everything_SetSearchW(query: *const u16);
    fn Everything_SetRequestFlags(flags: u32);
    fn Everything_SetOffset(offset: u32);
    fn Everything_SetMax(count: u32);
    fn Everything_SetSort(sort: u32);
    fn Everything_QueryW(wait: i32) -> i32;
    fn Everything_GetNumResults() -> u32;
    fn Everything_GetTotResults() -> u32;
    fn Everything_GetResultFullPathNameW(index: u32, buffer: *mut u16, length: u32) -> u32;
    fn Everything_IsFolderResult(index: u32) -> i32;
    fn Everything_GetResultSize(index: u32, size: *mut i64) -> i32;
    fn Everything_GetLastError() -> u32;
    fn Everything_CleanUp();
}

// RAII makes all early returns release SDK resources before worker exit.
struct Client(Handle);
impl Drop for Client {
    fn drop(&mut self) {
        unsafe {
            Everything3_DestroyClient(self.0);
        }
    }
}
struct State(Handle);
impl Drop for State {
    fn drop(&mut self) {
        unsafe {
            Everything3_DestroySearchState(self.0);
        }
    }
}
struct List(Handle);
impl Drop for List {
    fn drop(&mut self) {
        unsafe {
            Everything3_DestroyResultList(self.0);
        }
    }
}

pub(super) fn search(request: &Request) -> SearchResult {
    // SAFETY: official C ABI, all strings are NUL-terminated UTF-16, handles stay
    // alive until after their dependents; bounded buffers use the SDK's unit sizes.
    unsafe {
        if !request.force_legacy {
            let mut handle = Everything3_ConnectW(std::ptr::null());
            if handle.is_null() {
                let instance = wide("1.5a");
                handle = Everything3_ConnectW(instance.as_ptr());
            }
            if !handle.is_null() {
                return sdk3(request, Client(handle));
            }
        }
        sdk2(request)
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

unsafe fn sdk3(request: &Request, client: Client) -> SearchResult {
    unsafe {
        let mut response = SearchResult::status("ready", "");
        response.backend = Some("SDK3".into());
        response.version = Some(format!(
            "{}.{}.{}.{}",
            Everything3_GetMajorVersion(client.0),
            Everything3_GetMinorVersion(client.0),
            Everything3_GetRevision(client.0),
            Everything3_GetBuildNumber(client.0)
        ));
        if Everything3_IsDBLoaded(client.0) == 0 {
            response.state = "indexing".into();
            response.message = "Everything 正在加载索引，请稍后刷新".into();
            return response;
        }
        if request.query.is_empty() {
            return response;
        }
        let handle = Everything3_CreateSearchState();
        if handle.is_null() {
            return sdk_error(Everything3_GetLastError());
        }
        let state = State(handle);
        let query = wide(&request.query);
        // Full path must be explicitly requested: SDK3 does not synthesize it
        // from separate name/path fields. Size is optional (not indexed => None).
        if Everything3_SetSearchTextW(state.0, query.as_ptr()) == 0
            || !set_viewport(state.0, request.offset as usize, request.limit as usize)
            || Everything3_AddSearchPropertyRequest(state.0, 240) == 0
            || Everything3_AddSearchPropertyRequest(state.0, 2) == 0
            || Everything3_AddSearchSort(state.0, 0, 1) == 0
            || Everything3_AddSearchSort(state.0, 1, 1) == 0
        {
            return sdk_error(Everything3_GetLastError());
        }
        let handle = Everything3_Search(client.0, state.0);
        if handle.is_null() {
            return sdk_error(Everything3_GetLastError());
        }
        let list = List(handle);
        response.total = Everything3_GetResultListCount(list.0) as u64;
        response.offset = request.offset;
        for index in 0..Everything3_GetResultListViewportCount(list.0).min(request.limit as usize) {
            let mut buffer = vec![0u16; 32768];
            let length = Everything3_GetResultFullPathNameW(
                list.0,
                index,
                buffer.as_mut_ptr(),
                buffer.len(),
            );
            if length == 0 || length >= buffer.len() {
                return SearchResult::status("error", "无法读取完整文件路径，请缩小搜索范围后重试");
            }
            let path = String::from_utf16_lossy(&buffer[..length]);
            let is_dir = Everything3_IsFolderResult(list.0, index) != 0;
            let size = Everything3_GetResultSize(list.0, index);
            response.items.push(FileEntry::new(
                path,
                is_dir,
                (!is_dir && size != u64::MAX).then_some(size),
            ));
        }
        response
    }
}

unsafe fn set_viewport(state: Handle, offset: usize, count: usize) -> bool {
    unsafe {
        // SDK3 3.0.1.12 setters update the state but incorrectly return FALSE.
        // Verify their postconditions through getters instead of that return value.
        Everything3_SetSearchViewportOffset(state, offset);
        Everything3_SetSearchViewportCount(state, count);
        Everything3_GetSearchViewportOffset(state) == offset
            && Everything3_GetSearchViewportCount(state) == count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn official_sdk_viewport_is_bounded_for_first_and_subsequent_pages() {
        unsafe {
            let state = State(Everything3_CreateSearchState());
            assert!(!state.0.is_null());
            assert!(set_viewport(state.0, 0, 50));
            assert!(set_viewport(state.0, 50, 50));
            assert!(set_viewport(state.0, 100, 20));
        }
    }
}

unsafe fn sdk2(request: &Request) -> SearchResult {
    unsafe {
        let major = Everything_GetMajorVersion();
        if major == 0 {
            return SearchResult::status(
                "unavailable",
                "尚未连接到 Everything，请安装或启动 Everything 普通版。",
            );
        }
        let mut response = SearchResult::status("ready", "");
        response.backend = Some("SDK2".into());
        response.version = Some(format!(
            "{major}.{}.{}.{}",
            Everything_GetMinorVersion(),
            Everything_GetRevision(),
            Everything_GetBuildNumber()
        ));
        if Everything_IsDBLoaded() == 0 {
            response.state = "indexing".into();
            response.message = "Everything 正在加载索引，请稍后刷新".into();
            return response;
        }
        if request.query.is_empty() {
            return response;
        }
        let query = wide(&request.query);
        Everything_SetSearchW(query.as_ptr());
        Everything_SetRequestFlags(0x01 | 0x02 | 0x10);
        Everything_SetSort(1);
        Everything_SetOffset(request.offset);
        Everything_SetMax(request.limit);
        if Everything_QueryW(1) == 0 {
            return sdk_error(Everything_GetLastError());
        }
        response.total = Everything_GetTotResults() as u64;
        response.offset = request.offset;
        for index in 0..Everything_GetNumResults().min(request.limit) {
            let mut buffer = vec![0u16; 32768];
            let length =
                Everything_GetResultFullPathNameW(index, buffer.as_mut_ptr(), buffer.len() as u32)
                    as usize;
            if length == 0 || length >= buffer.len() {
                return SearchResult::status("error", "无法读取完整文件路径，请缩小搜索范围后重试");
            }
            let is_dir = Everything_IsFolderResult(index) != 0;
            let mut size = 0i64;
            let known = Everything_GetResultSize(index, &mut size) != 0 && size >= 0;
            response.items.push(FileEntry::new(
                String::from_utf16_lossy(&buffer[..length]),
                is_dir,
                (!is_dir && known).then_some(size as u64),
            ));
        }
        Everything_CleanUp();
        response
    }
}

fn sdk_error(code: u32) -> SearchResult {
    SearchResult::status(
        "error",
        format!("Everything 查询失败（0x{code:08X}），请重试"),
    )
}
