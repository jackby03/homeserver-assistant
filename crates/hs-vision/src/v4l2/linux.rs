//! Linux V4L2 capture source.

use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::convert::yuyv_to_luma;
use crate::{Frame, FrameSource, VisionError, VisionResult};

/// V4L2 fourcc for packed YUYV 4:2:2 (`'Y' 'U' 'Y' 'V'`).
const V4L2_PIX_FMT_YUYV: u32 = u32::from_le_bytes(*b"YUYV");
/// Device supports video capture.
const V4L2_CAP_VIDEO_CAPTURE: u32 = 0x0000_0001;
/// Device supports the `read(2)` / `write(2)` I/O methods.
const V4L2_CAP_READWRITE: u32 = 0x0100_0000;
/// When set, `device_caps` in `v4l2_capability` is valid.
const V4L2_CAP_DEVICE_CAPS: u32 = 0x8000_0000;
/// Single-planar video capture buffer type.
const V4L2_BUF_TYPE_VIDEO_CAPTURE: u32 = 1;
/// Let the driver pick the field order.
const V4L2_FIELD_ANY: u32 = 0;

/// How long a single `next_frame` waits for the camera.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(2);
/// Maximum time one `poll(2)` call blocks before the deadline is re-checked.
const POLL_SLICE: Duration = Duration::from_millis(100);

/// Size in bytes of `struct v4l2_capability`; identical on all word sizes.
const V4L2_CAPABILITY_SIZE: u32 = 104;
/// Size in bytes of `struct v4l2_format` on 64-bit Linux.
#[cfg(target_pointer_width = "64")]
const V4L2_FORMAT_SIZE: u32 = 208;
/// Size in bytes of `struct v4l2_format` on 32-bit Linux.
#[cfg(target_pointer_width = "32")]
const V4L2_FORMAT_SIZE: u32 = 204;
/// Offset of the `fmt` union inside `struct v4l2_format` on 64-bit Linux.
#[cfg(target_pointer_width = "64")]
const V4L2_FORMAT_UNION_OFFSET: usize = 8;
/// Offset of the `fmt` union inside `struct v4l2_format` on 32-bit Linux.
#[cfg(target_pointer_width = "32")]
const V4L2_FORMAT_UNION_OFFSET: usize = 4;

/// Offset of `v4l2_pix_format.width` inside the `v4l2_format` union.
const PIX_WIDTH: usize = 0;
/// Offset of `v4l2_pix_format.height` inside the `v4l2_format` union.
const PIX_HEIGHT: usize = 4;
/// Offset of `v4l2_pix_format.pixelformat` inside the `v4l2_format` union.
const PIX_PIXEL_FORMAT: usize = 8;
/// Offset of `v4l2_pix_format.field` inside the `v4l2_format` union.
const PIX_FIELD: usize = 12;
/// Offset of `v4l2_pix_format.bytesperline` inside the `v4l2_format` union.
const PIX_BYTES_PER_LINE: usize = 16;
/// Offset of `v4l2_pix_format.sizeimage` inside the `v4l2_format` union.
const PIX_SIZE_IMAGE: usize = 20;

/// `_IOC_READ` from `asm-generic/ioctl.h`.
const IOC_READ: u32 = 2;
/// `_IOC_READ | _IOC_WRITE` from `asm-generic/ioctl.h`.
const IOC_READ_WRITE: u32 = 3;

/// Encode a V4L2 ioctl request number (`_IOC` macro layout).
const fn v4l2_ioc(direction: u32, number: u32, size: u32) -> libc::c_ulong {
    ((direction as libc::c_ulong) << 30)
        | ((size as libc::c_ulong) << 16)
        | ((b'V' as libc::c_ulong) << 8)
        | (number as libc::c_ulong)
}

/// `VIDIOC_QUERYCAP`.
const VIDIOC_QUERYCAP: libc::c_ulong = v4l2_ioc(IOC_READ, 0, V4L2_CAPABILITY_SIZE);
/// `VIDIOC_G_FMT`.
const VIDIOC_G_FMT: libc::c_ulong = v4l2_ioc(IOC_READ_WRITE, 4, V4L2_FORMAT_SIZE);
/// `VIDIOC_S_FMT`.
const VIDIOC_S_FMT: libc::c_ulong = v4l2_ioc(IOC_READ_WRITE, 5, V4L2_FORMAT_SIZE);

/// Aligned raw storage for `struct v4l2_format`.
///
/// `struct v4l2_format` is `{ __u32 type; union { ... } fmt; }` and the union is
/// 200 bytes. Using an explicitly sized and aligned byte buffer avoids
/// replicating the kernel union (which contains platform-sized pointers) in
/// Rust while still passing a layout-compatible pointer to `ioctl`.
#[cfg(target_pointer_width = "64")]
#[repr(C, align(8))]
struct FormatBuffer([u8; V4L2_FORMAT_SIZE as usize]);

/// Aligned raw storage for `struct v4l2_format`.
#[cfg(target_pointer_width = "32")]
#[repr(C, align(4))]
struct FormatBuffer([u8; V4L2_FORMAT_SIZE as usize]);

impl FormatBuffer {
    /// Zeroed buffer with room for the `type` field and the format union.
    fn new() -> Self {
        Self([0u8; V4L2_FORMAT_SIZE as usize])
    }

    /// Full buffer bytes (header and union).
    fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.0
    }

    /// Format union bytes (everything after the `type` field).
    fn union(&self) -> &[u8] {
        &self.0[V4L2_FORMAT_UNION_OFFSET..]
    }

    /// Mutable format union bytes.
    fn union_mut(&mut self) -> &mut [u8] {
        &mut self.0[V4L2_FORMAT_UNION_OFFSET..]
    }

    /// Raw pointer handed to `ioctl`.
    fn as_mut_ptr(&mut self) -> *mut libc::c_void {
        self.0.as_mut_ptr().cast::<libc::c_void>()
    }
}

/// Format and buffer sizes negotiated with the driver.
#[derive(Debug, Clone, Copy)]
struct NegotiatedFormat {
    width: u32,
    height: u32,
    stride: usize,
    read_size: usize,
}

impl NegotiatedFormat {
    /// Number of bytes needed for one full frame.
    fn frame_bytes(&self) -> usize {
        self.stride.saturating_mul(self.height as usize)
    }
}

/// Linux V4L2 capture source.
///
/// Opens a video4linux2 device, negotiates packed YUYV at the requested
/// resolution and captures frames through the `read(2)` interface. If the
/// driver only exposes the streaming (MMAP) interface, opening fails with
/// [`VisionError::Unavailable`] and a message naming the limitation instead of
/// returning corrupted frames.
#[derive(Debug)]
pub struct V4l2Source {
    descriptor: OwnedFd,
    device: String,
    format: NegotiatedFormat,
    read_size: usize,
    buffer: Vec<u8>,
}

impl V4l2Source {
    /// Open a V4L2 capture device.
    ///
    /// `device` is a device node such as `/dev/video0`. The negotiated
    /// resolution may differ from `width` x `height` when the driver rounds it;
    /// use [`V4l2Source::width`] and [`V4l2Source::height`] for the effective
    /// values.
    ///
    /// # Errors
    ///
    /// Returns [`VisionError::Unavailable`] when the device cannot be opened,
    /// is not a capture device, does not support `read(2)` capture or has no
    /// usable YUYV format; returns [`VisionError::InvalidFrame`] for invalid
    /// requested dimensions.
    pub fn open(device: impl AsRef<Path>, width: u32, height: u32) -> VisionResult<Self> {
        crate::frame::validate_dimensions(width, height)?;
        let device_path = device.as_ref();
        let device_string = device_path.to_string_lossy().into_owned();
        let c_path = CString::new(device_path.as_os_str().as_bytes()).map_err(|_| {
            VisionError::Config(format!("device path contains a NUL byte: {device_string}"))
        })?;

        let descriptor = open_device(&c_path, &device_string)?;
        let capabilities = query_capabilities(descriptor.as_raw_fd(), &device_string)?;
        if capabilities & V4L2_CAP_VIDEO_CAPTURE == 0 {
            return Err(VisionError::Unavailable(format!(
                "{device_string} is not a V4L2 video capture device"
            )));
        }
        if capabilities & V4L2_CAP_READWRITE == 0 {
            return Err(VisionError::Unavailable(format!(
                "{device_string} does not support read(2) capture; streaming-only (MMAP) devices are not implemented"
            )));
        }

        let format = negotiate_format(descriptor.as_raw_fd(), &device_string, width, height)?;
        let read_size = format.read_size.max(format.frame_bytes()).max(1);
        let buffer = vec![0u8; read_size];
        Ok(Self {
            descriptor,
            device: device_string,
            format,
            read_size,
            buffer,
        })
    }

    /// Device node this source captures from.
    #[must_use]
    pub fn device(&self) -> &str {
        &self.device
    }

    /// Negotiated frame width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.format.width
    }

    /// Negotiated frame height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.format.height
    }
}

#[async_trait]
impl FrameSource for V4l2Source {
    async fn next_frame(&mut self) -> VisionResult<Frame> {
        let descriptor = self.descriptor.as_raw_fd();
        let device = self.device.clone();
        let format = self.format;
        let read_size = self.read_size;
        let mut buffer = std::mem::take(&mut self.buffer);
        let captured = tokio::task::spawn_blocking(move || {
            let result = capture_once(descriptor, &device, &mut buffer, format);
            (buffer, result)
        })
        .await;
        let (buffer, result) = match captured {
            Ok(completed) => completed,
            Err(error) => {
                self.buffer = vec![0u8; read_size];
                return Err(VisionError::Source(format!(
                    "camera capture task failed: {error}"
                )));
            }
        };
        self.buffer = buffer;
        let luma = result?;
        Frame::new(format.width, format.height, luma)
    }

    fn description(&self) -> String {
        self.device.clone()
    }
}

/// Open the device node, returning an owned descriptor.
fn open_device(path: &CString, device: &str) -> VisionResult<OwnedFd> {
    // SAFETY: `path` is a valid NUL-terminated C string and no variadic mode
    // argument is required because O_CREAT is not set.
    let descriptor = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_NONBLOCK) };
    if descriptor < 0 {
        let error = std::io::Error::last_os_error();
        return Err(VisionError::Unavailable(format!(
            "cannot open camera {device}: {error}"
        )));
    }
    // SAFETY: the descriptor was just returned by `open` and is not owned
    // anywhere else, so transferring ownership to `OwnedFd` is sound.
    Ok(unsafe { OwnedFd::from_raw_fd(descriptor) })
}

/// Query the effective device capabilities.
fn query_capabilities(descriptor: libc::c_int, device: &str) -> VisionResult<u32> {
    let mut buffer = [0u8; V4L2_CAPABILITY_SIZE as usize];
    // SAFETY: `buffer` is at least `sizeof(struct v4l2_capability)` bytes, is
    // live for the duration of the call and the request encodes that size.
    let result = unsafe {
        libc::ioctl(
            descriptor,
            VIDIOC_QUERYCAP,
            buffer.as_mut_ptr().cast::<libc::c_void>(),
        )
    };
    if result < 0 {
        let error = std::io::Error::last_os_error();
        return Err(VisionError::Unavailable(format!(
            "VIDIOC_QUERYCAP failed for {device}: {error}"
        )));
    }
    let capabilities = read_u32(&buffer, 84);
    if capabilities & V4L2_CAP_DEVICE_CAPS != 0 {
        return Ok(read_u32(&buffer, 88));
    }
    Ok(capabilities)
}

/// Negotiate packed YUYV at the requested resolution and read back the result.
fn negotiate_format(
    descriptor: libc::c_int,
    device: &str,
    width: u32,
    height: u32,
) -> VisionResult<NegotiatedFormat> {
    let mut requested = FormatBuffer::new();
    write_u32(requested.bytes_mut(), 0, V4L2_BUF_TYPE_VIDEO_CAPTURE);
    let request = requested.union_mut();
    write_u32(request, PIX_WIDTH, width);
    write_u32(request, PIX_HEIGHT, height);
    write_u32(request, PIX_PIXEL_FORMAT, V4L2_PIX_FMT_YUYV);
    write_u32(request, PIX_FIELD, V4L2_FIELD_ANY);
    // SAFETY: `requested` is a live, aligned `struct v4l2_format` of exactly the
    // size encoded in the ioctl request.
    let result = unsafe { libc::ioctl(descriptor, VIDIOC_S_FMT, requested.as_mut_ptr()) };
    if result < 0 {
        let error = std::io::Error::last_os_error();
        return Err(VisionError::Unavailable(format!(
            "VIDIOC_S_FMT failed for {device}: {error}"
        )));
    }

    let mut negotiated = FormatBuffer::new();
    // SAFETY: `negotiated` is a live, aligned `struct v4l2_format` of exactly
    // the size encoded in the ioctl request.
    let result = unsafe { libc::ioctl(descriptor, VIDIOC_G_FMT, negotiated.as_mut_ptr()) };
    if result < 0 {
        let error = std::io::Error::last_os_error();
        return Err(VisionError::Unavailable(format!(
            "VIDIOC_G_FMT failed for {device}: {error}"
        )));
    }

    let format = negotiated.union();
    let actual_width = read_u32(format, PIX_WIDTH);
    let actual_height = read_u32(format, PIX_HEIGHT);
    let pixel_format = read_u32(format, PIX_PIXEL_FORMAT);
    let bytes_per_line = read_u32(format, PIX_BYTES_PER_LINE) as usize;
    let size_image = read_u32(format, PIX_SIZE_IMAGE) as usize;

    if actual_width == 0 || actual_height == 0 {
        return Err(VisionError::Unavailable(format!(
            "{device} negotiated an empty frame size"
        )));
    }
    if pixel_format != V4L2_PIX_FMT_YUYV {
        return Err(VisionError::Unavailable(format!(
            "{device} negotiated unsupported pixel format {pixel_format:#010x}"
        )));
    }
    crate::frame::validate_dimensions(actual_width, actual_height)?;

    let minimum_stride = (actual_width as usize).saturating_mul(2);
    let stride = if bytes_per_line == 0 {
        minimum_stride
    } else {
        bytes_per_line
    };
    if stride < minimum_stride {
        return Err(VisionError::Unavailable(format!(
            "{device} reported a YUYV stride of {stride} bytes for {actual_width} pixels"
        )));
    }

    Ok(NegotiatedFormat {
        width: actual_width,
        height: actual_height,
        stride,
        read_size: size_image.max(stride.saturating_mul(actual_height as usize)),
    })
}

/// Capture one frame and convert it to a grayscale luma plane.
fn capture_once(
    descriptor: libc::c_int,
    device: &str,
    buffer: &mut [u8],
    format: NegotiatedFormat,
) -> VisionResult<Vec<u8>> {
    let deadline = Instant::now() + CAPTURE_TIMEOUT;
    loop {
        if Instant::now() >= deadline {
            return Err(VisionError::Unavailable(format!(
                "timed out waiting for a frame from {device}"
            )));
        }

        let timeout_ms = POLL_SLICE.as_millis() as libc::c_int;
        let mut poll_descriptor = libc::pollfd {
            fd: descriptor,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid `pollfd` is passed with `nfds = 1`.
        let ready = unsafe { libc::poll(&mut poll_descriptor, 1, timeout_ms) };
        if ready < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(VisionError::Unavailable(format!(
                "poll on {device} failed: {error}"
            )));
        }
        if ready == 0 {
            continue;
        }

        // SAFETY: `buffer` is a valid writable allocation of `buffer.len()`
        // bytes and the descriptor is owned by the caller.
        let read = unsafe {
            libc::read(
                descriptor,
                buffer.as_mut_ptr().cast::<libc::c_void>(),
                buffer.len(),
            )
        };
        if read < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted
                || error.kind() == std::io::ErrorKind::WouldBlock
            {
                continue;
            }
            return Err(VisionError::Unavailable(format!(
                "read from {device} failed: {error}"
            )));
        }

        let read = read as usize;
        let required = format.frame_bytes();
        if read < required {
            return Err(VisionError::Source(format!(
                "short frame from {device}: {read} of {required} bytes"
            )));
        }
        return yuyv_to_luma(format.width, format.height, format.stride, &buffer[..read]);
    }
}

/// Write a native-endian `u32` into a byte buffer at `offset`.
fn write_u32(buffer: &mut [u8], offset: usize, value: u32) {
    if let Some(slot) = buffer.get_mut(offset..offset + 4) {
        slot.copy_from_slice(&value.to_ne_bytes());
    }
}

/// Read a native-endian `u32` from a byte buffer at `offset`.
fn read_u32(buffer: &[u8], offset: usize) -> u32 {
    match buffer.get(offset..offset + 4) {
        Some(slot) => u32::from_ne_bytes([slot[0], slot[1], slot[2], slot[3]]),
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctl_requests_match_the_v4l2_abi() {
        // Values from the Linux UAPI headers for 64-bit builds.
        assert_eq!(VIDIOC_QUERYCAP, 0x8068_5600);
        #[cfg(target_pointer_width = "64")]
        {
            assert_eq!(VIDIOC_G_FMT, 0xC0D0_5604);
            assert_eq!(VIDIOC_S_FMT, 0xC0D0_5605);
        }
    }

    #[test]
    fn pixel_format_fourcc_is_yuyv() {
        assert_eq!(V4L2_PIX_FMT_YUYV, 0x5659_5559);
    }

    #[test]
    fn format_buffer_matches_the_kernel_struct_size() {
        assert_eq!(std::mem::size_of::<FormatBuffer>() as u32, V4L2_FORMAT_SIZE);
        assert_eq!(
            V4L2_FORMAT_UNION_OFFSET,
            if cfg!(target_pointer_width = "64") {
                8
            } else {
                4
            }
        );
    }

    #[test]
    fn u32_helpers_handle_out_of_bounds_offsets() {
        let mut buffer = [0u8; 8];
        write_u32(&mut buffer, 0, 0x1234_5678);
        assert_eq!(read_u32(&buffer, 0), 0x1234_5678);
        write_u32(&mut buffer, 6, 0xffff_ffff);
        assert_eq!(read_u32(&buffer, 6), 0);
    }
}
