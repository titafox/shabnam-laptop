import CoreMedia
import CoreVideo
import Foundation
import VideoToolbox

/// The Mac's hardware H.264 encoder, tuned for a fast cable and crisp text. Each encoded frame is
/// handed to `output` as an Annex-B byte string ready to be written to the socket.
final class Encoder {
    private let session: VTCompressionSession
    private let output: (Data) -> Void

    init?(width: Int, height: Int, output: @escaping (Data) -> Void) {
        var created: VTCompressionSession?
        let r = VTCompressionSessionCreate(allocator: nil, width: Int32(width), height: Int32(height), codecType: kCMVideoCodecType_H264,
                                           encoderSpecification: nil, imageBufferAttributes: nil, compressedDataAllocator: nil,
                                           outputCallback: nil, refcon: nil, compressionSessionOut: &created)
        guard r == 0, let s = created else {
            log("encoder create failed: \(r)")
            return nil
        }
        VTSessionSetProperty(s, key: kVTCompressionPropertyKey_RealTime, value: kCFBooleanTrue)
        VTSessionSetProperty(s, key: kVTCompressionPropertyKey_ProfileLevel, value: kVTProfileLevel_H264_High_AutoLevel)
        VTSessionSetProperty(s, key: kVTCompressionPropertyKey_AllowFrameReordering, value: kCFBooleanFalse) // no B-frames: no added delay
        VTSessionSetProperty(s, key: kVTCompressionPropertyKey_MaxKeyFrameInterval, value: 3600 as CFNumber)
        // The cable carries about 300 Mbit/s, so spend bits freely: a high rate target, and a ceiling
        // on how coarsely any frame may be quantised (lower = sharper) so text stays crisp in motion.
        VTSessionSetProperty(s, key: kVTCompressionPropertyKey_AverageBitRate, value: 120_000_000 as CFNumber)
        let qp = VTSessionSetProperty(s, key: kVTCompressionPropertyKey_MaxAllowedFrameQP, value: 18 as CFNumber)
        log("quality ceiling (max QP 18): \(qp == 0 ? "on" : "not supported, status \(qp)")")
        VTSessionSetProperty(s, key: kVTCompressionPropertyKey_ExpectedFrameRate, value: 60 as CFNumber)
        VTCompressionSessionPrepareToEncodeFrames(s)
        session = s
        self.output = output
    }

    func encode(_ pixelBuffer: CVPixelBuffer, at pts: CMTime, forceKeyframe: Bool) {
        let props = forceKeyframe ? [kVTEncodeFrameOptionKey_ForceKeyFrame: true] as CFDictionary : nil
        let output = self.output
        VTCompressionSessionEncodeFrame(session, imageBuffer: pixelBuffer, presentationTimeStamp: pts, duration: .invalid,
                                        frameProperties: props, infoFlagsOut: nil) { status, _, sampleBuffer in
            if status == 0, let sampleBuffer, let data = AnnexB.frame(from: sampleBuffer) { output(data) }
        }
    }

    /// Finishes pending frames and releases the hardware encoder.
    func invalidate() {
        VTCompressionSessionCompleteFrames(session, untilPresentationTimeStamp: .invalid)
        VTCompressionSessionInvalidate(session)
    }
}

/// Turns the encoder's output (length-prefixed units) into the start-code byte stream the laptop expects.
enum AnnexB {
    private static let startCode: [UInt8] = [0, 0, 0, 1]

    static func frame(from sb: CMSampleBuffer) -> Data? {
        var out = Data()
        let attachments = CMSampleBufferGetSampleAttachmentsArray(sb, createIfNecessary: false) as? [[CFString: Any]]
        let isKey = !((attachments?.first?[kCMSampleAttachmentKey_NotSync] as? Bool) ?? false)
        // Keyframes carry the stream's parameter sets (SPS, PPS) in front, so the decoder can start there.
        if isKey, let format = CMSampleBufferGetFormatDescription(sb) {
            var count = 0
            CMVideoFormatDescriptionGetH264ParameterSetAtIndex(format, parameterSetIndex: 0, parameterSetPointerOut: nil,
                                                               parameterSetSizeOut: nil, parameterSetCountOut: &count, nalUnitHeaderLengthOut: nil)
            for i in 0..<count {
                var p: UnsafePointer<UInt8>?
                var n = 0
                if CMVideoFormatDescriptionGetH264ParameterSetAtIndex(format, parameterSetIndex: i, parameterSetPointerOut: &p,
                                                                      parameterSetSizeOut: &n, parameterSetCountOut: nil, nalUnitHeaderLengthOut: nil) == 0, let p {
                    out.append(contentsOf: startCode)
                    out.append(p, count: n)
                }
            }
        }
        guard let block = CMSampleBufferGetDataBuffer(sb) else { return nil }
        let total = CMBlockBufferGetDataLength(block)
        var raw = [UInt8](repeating: 0, count: total)
        guard CMBlockBufferCopyDataBytes(block, atOffset: 0, dataLength: total, destination: &raw) == 0 else { return nil }
        var off = 0
        while off + 4 <= total {
            let n = Int(raw[off]) << 24 | Int(raw[off + 1]) << 16 | Int(raw[off + 2]) << 8 | Int(raw[off + 3])
            off += 4
            guard n > 0, off + n <= total else { break }
            out.append(contentsOf: startCode)
            out.append(contentsOf: raw[off..<off + n])
            off += n
        }
        // An "access unit delimiter" after the frame tells the receiver's parser the frame is complete,
        // so it is decoded now instead of when the next frame arrives.
        out.append(contentsOf: startCode + [0x09, 0xF0])
        return out
    }
}
