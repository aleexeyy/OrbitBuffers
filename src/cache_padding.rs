#[cfg(target_pointer_width = "64")]
#[repr(align(64))]
/// Wraps a value and aligns it to a cache line to reduce false sharing.
pub struct CachePadded<T>(pub T);

#[cfg(target_pointer_width = "32")]
#[repr(align(32))]
/// Wraps a value and aligns it to a cache line to reduce false sharing.
pub struct CachePadded<T>(pub T);

#[cfg(target_pointer_width = "16")]
#[repr(align(16))]
/// Wraps a value and aligns it to a cache line to reduce false sharing.
pub struct CachePadded<T>(pub T);
