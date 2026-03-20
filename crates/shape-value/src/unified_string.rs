use std::alloc::{self, Layout};
use std::sync::atomic::AtomicU32;
use crate::tags;

pub const UNIFIED_STRING_DATA_OFFSET: i32 = 8;
pub const UNIFIED_STRING_LEN_OFFSET: i32 = 16;

#[repr(C)]
pub struct UnifiedString {
    pub kind: u16,
    pub flags: u8,
    pub _reserved: u8,
    pub refcount: AtomicU32,
    pub data: *const u8,
    pub len: u64,
    pub cap: u64,
}

unsafe impl Send for UnifiedString {}
unsafe impl Sync for UnifiedString {}

const _: () = {
    assert!(std::mem::size_of::<UnifiedString>() == 32);
    assert!(std::mem::offset_of!(UnifiedString, data) == 8);
    assert!(std::mem::offset_of!(UnifiedString, len) == 16);
    assert!(std::mem::offset_of!(UnifiedString, cap) == 24);
};

impl UnifiedString {
    pub fn from_str(s: &str) -> Self {
        if s.is_empty() { return Self { kind: tags::HEAP_KIND_STRING as u16, flags: 0, _reserved: 0, refcount: AtomicU32::new(1), data: std::ptr::null(), len: 0, cap: 0 }; }
        let len = s.len();
        let layout = Layout::array::<u8>(len).unwrap();
        let data = unsafe { alloc::alloc(layout) };
        if data.is_null() { alloc::handle_alloc_error(layout); }
        unsafe { std::ptr::copy_nonoverlapping(s.as_ptr(), data, len); }
        Self { kind: tags::HEAP_KIND_STRING as u16, flags: 0, _reserved: 0, refcount: AtomicU32::new(1), data, len: len as u64, cap: len as u64 }
    }
    pub fn from_string(s: String) -> Self {
        if s.is_empty() { return Self { kind: tags::HEAP_KIND_STRING as u16, flags: 0, _reserved: 0, refcount: AtomicU32::new(1), data: std::ptr::null(), len: 0, cap: 0 }; }
        let mut s = s.into_bytes();
        let len = s.len(); let cap = s.capacity(); let data = s.as_mut_ptr();
        std::mem::forget(s);
        Self { kind: tags::HEAP_KIND_STRING as u16, flags: 0, _reserved: 0, refcount: AtomicU32::new(1), data, len: len as u64, cap: cap as u64 }
    }
    #[inline] pub fn as_str(&self) -> &str {
        if self.data.is_null() || self.len == 0 { return ""; }
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(self.data, self.len as usize)) }
    }
    #[inline] pub fn len(&self) -> usize { self.len as usize }
    #[inline] pub fn is_empty(&self) -> bool { self.len == 0 }
    #[inline] pub fn heap_box(self) -> u64 { let ptr = Box::into_raw(Box::new(self)); tags::make_unified_heap(ptr as *const u8) }
    #[inline] pub unsafe fn from_heap_bits(bits: u64) -> &'static Self { let ptr = tags::unified_heap_ptr(bits) as *const Self; unsafe { &*ptr } }
    pub unsafe fn heap_drop(bits: u64) { let ptr = tags::unified_heap_ptr(bits) as *mut Self; unsafe { drop(Box::from_raw(ptr)) }; }
}

impl Drop for UnifiedString {
    fn drop(&mut self) {
        if !self.data.is_null() && self.cap > 0 {
            let layout = Layout::array::<u8>(self.cap as usize).unwrap();
            unsafe { alloc::dealloc(self.data as *mut u8, layout) };
        }
    }
}
