//! V4L2 camera capture source.
//!
//! On Linux the [`V4l2Source`] adapter opens a Video4Linux2 device, negotiates
//! packed YUYV frames and uses the `read(2)` capture interface. On every other
//! platform a stub type is exposed so the binary and the rest of the workspace
//! still compile on development machines; opening a camera there always fails
//! with [`crate::VisionError::Unavailable`].

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::V4l2Source;

#[cfg(not(target_os = "linux"))]
mod stub;
#[cfg(not(target_os = "linux"))]
pub use stub::V4l2Source;
