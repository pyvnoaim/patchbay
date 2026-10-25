//! H.264 for the remote desktop's graphics pipeline, decoded by the OS's own decoder:
//! VideoToolbox on macOS, Media Foundation on Windows. The codec licence is the
//! platform's, nothing in C is linked, and the decode runs on the video hardware where
//! there is some. IronRDP takes the planes and does the colour conversion, and AVC444's
//! recombining, itself.
//!
//! Elsewhere there is no decoder and none is advertised: the server then picks one of
//! the graphics pipeline's other codecs, all of which IronRDP decodes in Rust.
//!
//! VideoToolbox is ours, Media Foundation is IronRDP's. It hands back the YUV planes as
//! decoded, never asking VideoToolbox for a pixel format, because a requested one can
//! add a range conversion IronRDP then applies again; IronRDP does the colour transform
//! and AVC444's recombining. A decode error ends the session (the graphics pipeline
//! passes it up), so there is no recovering mid-stream. The picture is copied into a
//! block CoreMedia owns, because nothing says VideoToolbox lets go of a sample when the
//! decode returns. Never an H.264 library in C.

use ironrdp_egfx::decode::H264Decoder;

/// This platform's decoder, or `None` where there isn't one.
pub fn decoder() -> Option<Box<dyn H264Decoder>> {
    #[cfg(target_os = "macos")]
    return Some(Box::new(videotoolbox::Decoder::default()));
    #[cfg(target_os = "windows")]
    return ironrdp_egfx_native::MediaFoundationH264Decoder::new(
        ironrdp_egfx_native::YuvMatrix::Bt709Full,
    )
    .ok()
    .map(|d| Box::new(d) as Box<dyn H264Decoder>);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return None;
}

/// The NAL units of one access unit, as an Annex B byte stream (start codes) or in AVC
/// format (each unit after a 4-byte big-endian length), the two forms
/// [`H264Decoder::decode`] takes. Empty units are dropped; a length running past the end
/// ends the walk rather than reading past it. Which framing it is can only be guessed: an
/// AVC stream whose first unit is 1 or 256-511 bytes long opens like a start code. Windows
/// and xrdp both send Annex B, which is what the guess favours.
#[cfg(any(target_os = "macos", test))]
pub fn nal_units(data: &[u8]) -> Vec<&[u8]> {
    let annex_b = data.starts_with(&[0, 0, 1]) || data.starts_with(&[0, 0, 0, 1]);
    let mut units = Vec::new();
    if annex_b {
        let mut start = None;
        let mut i = 0;
        while i + 3 <= data.len() {
            if data[i..i + 3] == [0, 0, 1] {
                if let Some(s) = start {
                    // A four-byte start code leaves its leading zero on the unit before.
                    let end = if i > s && data[i - 1] == 0 { i - 1 } else { i };
                    units.push(&data[s..end]);
                }
                i += 3;
                start = Some(i);
            } else {
                i += 1;
            }
        }
        if let Some(s) = start {
            units.push(&data[s..]);
        }
    } else {
        let mut rest = data;
        while rest.len() >= 4 {
            let len = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
            let Some(unit) = rest.get(4..4 + len) else {
                break;
            };
            units.push(unit);
            rest = &rest[4 + len..];
        }
    }
    units.retain(|u| !u.is_empty());
    units
}

#[cfg(target_os = "macos")]
mod videotoolbox {
    use super::nal_units;
    use ironrdp_egfx::decode::{
        DecodedFrame, DecoderError, DecoderResult, H264Decoder, Yuv420Frame,
    };
    use objc2_core_foundation::{CFDictionary, CFRetained, CFString, CFType};
    use objc2_core_media::{
        kCMBlockBufferAssureMemoryNowFlag, CMBlockBuffer, CMFormatDescription, CMSampleBuffer,
        CMVideoFormatDescriptionCreateFromH264ParameterSets,
    };
    use objc2_core_video::{
        kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
        kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange, kCVPixelFormatType_420YpCbCr8Planar,
        kCVPixelFormatType_420YpCbCr8PlanarFullRange, CVImageBuffer,
        CVPixelBufferGetBaseAddressOfPlane, CVPixelBufferGetBytesPerRowOfPlane,
        CVPixelBufferGetHeight, CVPixelBufferGetHeightOfPlane, CVPixelBufferGetPixelFormatType,
        CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
        CVPixelBufferUnlockBaseAddress,
    };
    use objc2_video_toolbox::{
        kVTVideoDecoderSpecification_EnableHardwareAcceleratedVideoDecoder, VTDecodeFrameFlags,
        VTDecodeInfoFlags, VTDecompressionOutputCallbackRecord, VTDecompressionSession,
    };
    use std::ffi::c_void;
    use std::ptr::{self, NonNull};

    const SPS: u8 = 7;
    const PPS: u8 = 8;
    const AUD: u8 = 9;

    /// One picture, or why there isn't one, written by [`output`] while
    /// `VTDecompressionSessionDecodeFrame` runs.
    type Slot = Option<Result<Yuv420Frame, String>>;

    #[derive(Default)]
    pub struct Decoder {
        sps: Vec<u8>,
        pps: Vec<u8>,
        session: Option<Session>,
    }

    struct Session {
        vt: CFRetained<VTDecompressionSession>,
        format: CFRetained<CMFormatDescription>,
    }

    impl Drop for Session {
        fn drop(&mut self) {
            unsafe { self.vt.invalidate() };
        }
    }

    // A decompression session may be used from any thread, one call at a time; the
    // graphics pipeline only ever calls a decoder from the session thread that owns it.
    unsafe impl Send for Decoder {}

    fn err(what: &str, status: i32) -> DecoderError {
        DecoderError::msg(format!("{what} failed ({status})"))
    }

    impl Session {
        fn new(sps: &[u8], pps: &[u8]) -> DecoderResult<Self> {
            let pointers = [NonNull::from(&sps[0]), NonNull::from(&pps[0])];
            let sizes = [sps.len(), pps.len()];
            let mut format: *const CMFormatDescription = ptr::null();
            let status = unsafe {
                CMVideoFormatDescriptionCreateFromH264ParameterSets(
                    None,
                    2,
                    NonNull::from(&pointers[0]),
                    NonNull::from(&sizes[0]),
                    4,
                    NonNull::from(&mut format),
                )
            };
            let format = NonNull::new(format.cast_mut())
                .filter(|_| status == 0)
                .ok_or_else(|| err("reading the H.264 parameter sets", status))?;
            let format = unsafe { CFRetained::from_raw(format) };

            // Hardware where the Mac has it, software where it doesn't: "enable", not
            // "require".
            let yes: &CFType = objc2_core_foundation::CFBoolean::new(true);
            let spec = CFDictionary::<CFString, CFType>::from_slices(
                &[unsafe { kVTVideoDecoderSpecification_EnableHardwareAcceleratedVideoDecoder }],
                &[yes],
            );
            let callback = VTDecompressionOutputCallbackRecord {
                decompressionOutputCallback: Some(output),
                decompressionOutputRefCon: ptr::null_mut(),
            };
            let mut vt: *mut VTDecompressionSession = ptr::null_mut();
            // No pixel format asked for: the decoder's own is the coded samples as they
            // are, where asking for one could add a range conversion IronRDP then does
            // again. `output` reads either 4:2:0 layout.
            let status = unsafe {
                VTDecompressionSession::create(
                    None,
                    &format,
                    Some(spec.as_opaque()),
                    None,
                    &callback,
                    NonNull::from(&mut vt),
                )
            };
            let vt = NonNull::new(vt)
                .filter(|_| status == 0)
                .ok_or_else(|| err("starting the H.264 decoder", status))?;
            Ok(Self {
                vt: unsafe { CFRetained::from_raw(vt) },
                format,
            })
        }

        /// One access unit in, one picture out. Synchronous: no asynchronous flag is
        /// passed, and the wait makes sure `output` has run before the slot is read.
        fn decode(&self, units: &[&[u8]]) -> DecoderResult<Yuv420Frame> {
            let mut sample = Vec::with_capacity(units.iter().map(|u| u.len() + 4).sum());
            for u in units {
                sample.extend_from_slice(&(u.len() as u32).to_be_bytes());
                sample.extend_from_slice(u);
            }

            // A block CoreMedia allocates and owns, with the picture copied in: nothing says
            // VideoToolbox lets go of a sample when the decode returns, so it must not point
            // at memory of ours.
            let mut block: *mut CMBlockBuffer = ptr::null_mut();
            let status = unsafe {
                CMBlockBuffer::create_with_memory_block(
                    None,
                    ptr::null_mut(),
                    sample.len(),
                    None,
                    ptr::null(),
                    0,
                    sample.len(),
                    kCMBlockBufferAssureMemoryNowFlag,
                    NonNull::from(&mut block),
                )
            };
            let block = NonNull::new(block)
                .filter(|_| status == 0)
                .ok_or_else(|| err("wrapping the H.264 picture", status))?;
            let block = unsafe { CFRetained::from_raw(block) };
            let status = unsafe {
                CMBlockBuffer::replace_data_bytes(
                    NonNull::from(&sample[0]).cast(),
                    &block,
                    0,
                    sample.len(),
                )
            };
            if status != 0 {
                return Err(err("wrapping the H.264 picture", status));
            }

            let mut buffer: *mut CMSampleBuffer = ptr::null_mut();
            let size = sample.len();
            let status = unsafe {
                CMSampleBuffer::create_ready(
                    None,
                    Some(&block),
                    Some(&self.format),
                    1,
                    0,
                    ptr::null(),
                    1,
                    &size,
                    NonNull::from(&mut buffer),
                )
            };
            let buffer = NonNull::new(buffer)
                .filter(|_| status == 0)
                .ok_or_else(|| err("wrapping the H.264 picture", status))?;
            let buffer = unsafe { CFRetained::from_raw(buffer) };

            let mut slot: Slot = None;
            // ponytail: no timeout; the decode runs in VideoToolbox's own service and a
            // stream that hung it would hang this session's thread. Media Foundation's
            // wrapper gives up after a second; the same here if a stream ever does that.
            let status = unsafe {
                self.vt.decode_frame(
                    &buffer,
                    VTDecodeFrameFlags(0),
                    (&mut slot as *mut Slot).cast(),
                    ptr::null_mut::<VTDecodeInfoFlags>(),
                )
            };
            unsafe { self.vt.wait_for_asynchronous_frames() };
            if status != 0 {
                return Err(err("decoding an H.264 picture", status));
            }
            match slot {
                Some(Ok(frame)) => Ok(frame),
                Some(Err(e)) => Err(DecoderError::msg(e)),
                None => Err(DecoderError::msg("the H.264 decoder returned no picture")),
            }
        }
    }

    /// VideoToolbox's output callback. `source` is the [`Slot`] the decode call passed in.
    unsafe extern "C-unwind" fn output(
        _: *mut c_void,
        source: *mut c_void,
        status: i32,
        _: VTDecodeInfoFlags,
        image: *mut CVImageBuffer,
        _: objc2_core_media::CMTime,
        _: objc2_core_media::CMTime,
    ) {
        let slot = &mut *source.cast::<Slot>();
        *slot = Some(match image.as_ref() {
            Some(image) if status == 0 => planes(image),
            _ => Err(format!("decoding an H.264 picture failed ({status})")),
        });
    }

    /// The picture's planes, copied out while the buffer is locked.
    fn planes(image: &CVImageBuffer) -> Result<Yuv420Frame, String> {
        let width = CVPixelBufferGetWidth(image);
        let height = CVPixelBufferGetHeight(image);
        let (Ok(w16), Ok(h16)) = (u16::try_from(width), u16::try_from(height)) else {
            return Err(format!("an H.264 picture of {width}x{height}"));
        };
        let format = CVPixelBufferGetPixelFormatType(image);
        let biplanar = [
            kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
            kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
        ]
        .contains(&format);
        let planar = [
            kCVPixelFormatType_420YpCbCr8Planar,
            kCVPixelFormatType_420YpCbCr8PlanarFullRange,
        ]
        .contains(&format);
        if !biplanar && !planar {
            return Err(format!("the H.264 decoder answered in format {format:#x}"));
        }

        if unsafe { CVPixelBufferLockBaseAddress(image, CVPixelBufferLockFlags::ReadOnly) } != 0 {
            return Err("the H.264 picture could not be read".into());
        }
        let cw = width.div_ceil(2);
        let ch = height.div_ceil(2);
        // `w` bytes from each of `h` rows, after checking the plane has them: the copy
        // below reads raw memory and the decoder's word on the size is all there is.
        let plane = |i: usize, w: usize, h: usize| -> Option<Vec<u8>> {
            let base = CVPixelBufferGetBaseAddressOfPlane(image, i).cast::<u8>();
            let stride = CVPixelBufferGetBytesPerRowOfPlane(image, i);
            if base.is_null() || stride < w || CVPixelBufferGetHeightOfPlane(image, i) < h {
                return None;
            }
            let mut out = Vec::with_capacity(w * h);
            for row in 0..h {
                out.extend_from_slice(unsafe {
                    std::slice::from_raw_parts(base.add(row * stride), w)
                });
            }
            Some(out)
        };
        let result = (|| {
            let y = plane(0, width, height)?;
            let (u, v) = if biplanar {
                // NV12: one plane of interleaved U and V, split as it is read.
                let base = CVPixelBufferGetBaseAddressOfPlane(image, 1).cast::<u8>();
                let stride = CVPixelBufferGetBytesPerRowOfPlane(image, 1);
                if base.is_null() || stride < cw * 2 || CVPixelBufferGetHeightOfPlane(image, 1) < ch
                {
                    return None;
                }
                let (mut u, mut v) = (Vec::with_capacity(cw * ch), Vec::with_capacity(cw * ch));
                for row in 0..ch {
                    let uv = unsafe { std::slice::from_raw_parts(base.add(row * stride), cw * 2) };
                    for &[cu, cv] in uv.as_chunks::<2>().0 {
                        u.push(cu);
                        v.push(cv);
                    }
                }
                (u, v)
            } else {
                (plane(1, cw, ch)?, plane(2, cw, ch)?)
            };
            Some((y, u, v))
        })();
        unsafe { CVPixelBufferUnlockBaseAddress(image, CVPixelBufferLockFlags::ReadOnly) };

        let (y, u, v) = result.ok_or("the H.264 picture could not be read")?;
        Yuv420Frame::new(w16, h16, y, width, u, v, cw).map_err(|e| e.to_string())
    }

    impl Decoder {
        fn picture(&mut self, data: &[u8]) -> DecoderResult<Yuv420Frame> {
            let mut picture = Vec::new();
            for unit in nal_units(data) {
                match unit[0] & 0x1f {
                    SPS if unit != self.sps.as_slice() => {
                        self.sps = unit.to_vec();
                        self.session = None;
                    }
                    PPS if unit != self.pps.as_slice() => {
                        self.pps = unit.to_vec();
                        self.session = None;
                    }
                    SPS | PPS | AUD => {}
                    _ => picture.push(unit),
                }
            }
            if picture.is_empty() {
                return Err(DecoderError::msg(
                    "an H.264 access unit with no picture in it",
                ));
            }
            if self.sps.is_empty() || self.pps.is_empty() {
                return Err(DecoderError::msg(
                    "an H.264 picture before its parameter sets",
                ));
            }
            if self.session.is_none() {
                self.session = Some(Session::new(&self.sps, &self.pps)?);
            }
            // An error here ends the session (the graphics pipeline passes it up), so there
            // is no recovering a decoder for the next picture.
            self.session.as_ref().unwrap().decode(&picture)
        }
    }

    impl H264Decoder for Decoder {
        fn decode(&mut self, data: &[u8]) -> DecoderResult<DecodedFrame> {
            Ok(self.picture(data)?.to_rgba())
        }

        fn decode_yuv420(&mut self, data: &[u8]) -> DecoderResult<Yuv420Frame> {
            self.picture(data)
        }

        fn supports_yuv420(&self) -> bool {
            true
        }

        fn reset(&mut self) {
            *self = Self::default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::nal_units;

    #[test]
    fn nal_units_split_both_framings() {
        let annex_b = [
            0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 0, 1, 0x65, 4, 5, 0,
        ];
        assert_eq!(
            nal_units(&annex_b),
            vec![&[0x67, 1, 2][..], &[0x68, 3], &[0x65, 4, 5, 0]]
        );

        let avc = [0, 0, 0, 2, 0x67, 9, 0, 0, 0, 1, 0x65];
        assert_eq!(nal_units(&avc), vec![&[0x67, 9][..], &[0x65]]);
    }

    #[test]
    fn a_length_past_the_end_stops_the_walk() {
        assert_eq!(nal_units(&[0, 0, 0, 9, 0x65, 1]), Vec::<&[u8]>::new());
        assert_eq!(
            nal_units(&[0, 0, 0, 2, 0x65, 0x88, 0, 0, 0, 7, 1]),
            vec![&[0x65, 0x88][..]]
        );
    }
}
