use std::sync::atomic::AtomicU32;
use crate::tags;

pub const UNIFIED_WRAPPER_KIND_OFFSET: i32 = 0;
pub const UNIFIED_WRAPPER_REFCOUNT_OFFSET: i32 = 4;
pub const UNIFIED_WRAPPER_INNER_OFFSET: i32 = 8;

#[repr(C)]
pub struct UnifiedWrapper {
    pub kind: u16,
    pub flags: u8,
    pub _reserved: u8,
    pub refcount: AtomicU32,
    pub inner: u64,
}

unsafe impl Send for UnifiedWrapper {}
unsafe impl Sync for UnifiedWrapper {}

const _: () = {
    assert!(std::mem::size_of::<UnifiedWrapper>() == 16);
    assert!(std::mem::offset_of!(UnifiedWrapper, kind) == 0);
    assert!(std::mem::offset_of!(UnifiedWrapper, refcount) == 4);
    assert!(std::mem::offset_of!(UnifiedWrapper, inner) == 8);
};

impl UnifiedWrapper {
    #[inline] pub fn new_ok(inner: u64) -> Self { Self { kind: tags::HEAP_KIND_OK as u16, flags: 0, _reserved: 0, refcount: AtomicU32::new(1), inner } }
    #[inline] pub fn new_err(inner: u64) -> Self { Self { kind: tags::HEAP_KIND_ERR as u16, flags: 0, _reserved: 0, refcount: AtomicU32::new(1), inner } }
    #[inline] pub fn new_some(inner: u64) -> Self { Self { kind: tags::HEAP_KIND_SOME as u16, flags: 0, _reserved: 0, refcount: AtomicU32::new(1), inner } }
    #[inline] pub fn is_ok(&self) -> bool { self.kind == tags::HEAP_KIND_OK as u16 }
    #[inline] pub fn is_err(&self) -> bool { self.kind == tags::HEAP_KIND_ERR as u16 }
    #[inline] pub fn is_some(&self) -> bool { self.kind == tags::HEAP_KIND_SOME as u16 }
    #[inline]
    pub fn heap_box(self) -> u64 {
        let ptr = Box::into_raw(Box::new(self));
        tags::make_unified_heap(ptr as *const u8)
    }
    #[inline]
    pub unsafe fn from_heap_bits(bits: u64) -> &'static Self {
        let ptr = tags::unified_heap_ptr(bits) as *const Self;
        unsafe { &*ptr }
    }
    pub unsafe fn heap_drop(bits: u64) {
        let ptr = tags::unified_heap_ptr(bits) as *mut Self;
        unsafe { drop(Box::from_raw(ptr)) };
    }
}

impl Drop for UnifiedWrapper {
    fn drop(&mut self) {
        let inner_bits = self.inner;
        if tags::is_tagged(inner_bits) && tags::get_tag(inner_bits) == tags::TAG_HEAP {
            if tags::is_unified_heap(inner_bits) {
                let ptr = tags::unified_heap_ptr(inner_bits);
                if !ptr.is_null() {
                    let rc = unsafe { (ptr.add(4)) as *const AtomicU32 };
                    let prev = unsafe { (*rc).fetch_sub(1, std::sync::atomic::Ordering::Release) };
                    if prev == 1 {
                        std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);
                        let kind = unsafe { *(ptr as *const u16) };
                        if kind == tags::HEAP_KIND_OK as u16 || kind == tags::HEAP_KIND_ERR as u16 || kind == tags::HEAP_KIND_SOME as u16 {
                            unsafe { UnifiedWrapper::heap_drop(inner_bits) };
                        } else if kind == tags::HEAP_KIND_STRING as u16 {
                            unsafe { crate::unified_string::UnifiedString::heap_drop(inner_bits) };
                        }
                    }
                }
            } else {
                let ptr = tags::get_payload(inner_bits) as *const crate::heap_value::HeapValue;
                if !ptr.is_null() { unsafe { std::sync::Arc::decrement_strong_count(ptr) }; }
            }
        }
    }
}
