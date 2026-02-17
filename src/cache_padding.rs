#[cfg(target_pointer_width = "64")]
#[repr(align(64))]
pub struct CachePadded<T>(pub T);

#[cfg(target_pointer_width = "32")]
#[repr(align(32))]
pub struct CachePadded<T>(pub T);

#[cfg(target_pointer_width = "16")]
#[repr(align(16))]
pub struct CachePadded<T>(pub T);
