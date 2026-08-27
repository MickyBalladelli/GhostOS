// Inventory: coverage_59_7.rs (legacy roadmap section 59).
use ghostos_platform_io::{BufferAccess, BufferDescriptor};
use ghostos_remote_display::{
    Av1RtpPacketizer, DisplayDaemon, DisplayError, DisplayGrant, DisplayRights, EncodedFrame,
    FrameDescriptor, FramePlane, FramePool, FrameState, NetworkFeedback, PixelFormat, WebRtcOffer,
};

fn buffer(length: u32) -> BufferDescriptor {
    BufferDescriptor { region: 1, offset: 0, length, access: BufferAccess::ReadOnly }
}

#[test]
fn display_pool_enforces_capture_encode_flight_order() {
    let descriptor = FrameDescriptor::new(
        1,
        4,
        2,
        PixelFormat::Rgba8888,
        90,
        &[FramePlane { buffer: buffer(32), stride_bytes: 16 }],
    )
    .unwrap();
    let mut pool = FramePool::<1>::new();
    let token = pool.register(descriptor).unwrap();
    let capture = pool.acquire_capture().unwrap();
    assert_eq!(pool.state(token), Ok(FrameState::Capturing));
    pool.submit_capture(capture.token, 2, 180).unwrap();
    let encode = pool.acquire_encode().unwrap();
    assert_eq!(pool.state(token), Ok(FrameState::Encoding));
    assert_eq!(pool.release(token), Err(DisplayError::InvalidState));
    pool.mark_in_flight(encode.token).unwrap();
    pool.release(token).unwrap();
    assert_eq!(pool.unregister(token).unwrap().frame_id, 2);
}

#[test]
fn remote_display_negotiates_capability_and_packetizes_av1() {
    let offer = WebRtcOffer {
        av1: true,
        av1_10_bit: false,
        max_width: 1280,
        max_height: 720,
        max_frames_per_second: 60,
        max_bitrate_bits_per_second: 1_000_000,
        mtu: 100,
    };
    let grant = DisplayGrant {
        principal: 7,
        display: 3,
        rights: DisplayRights::VIEW,
        expires_at_us: 100,
    };
    let mut daemon = DisplayDaemon::<1>::new(44);
    let (session, answer) = daemon.negotiate(grant, offer, 10).unwrap();
    assert_eq!(answer.ssrc, 44);
    assert!(daemon.authorize_input(session, 10).is_err());
    assert!(daemon.should_force_keyframe(session).unwrap());

    let frame = EncodedFrame {
        frame_id: 9,
        timestamp_90khz: 100,
        buffer: buffer(200),
        bytes: 150,
        keyframe: true,
        temporal_id: 0,
    };
    let mut packets = [ghostos_remote_display::RtpPacket {
        payload_type: 0,
        sequence: 0,
        timestamp: 0,
        ssrc: 0,
        marker: false,
        av1_aggregation_header: 0,
        payload: buffer(1),
    }; 3];
    assert_eq!(daemon.packetize(session, frame, 11, &mut packets).unwrap(), 2);
    assert_eq!(packets[0].sequence, 0);
    assert!(!packets[0].marker);
    assert!(packets[1].marker);
    assert!(!daemon.should_force_keyframe(session).unwrap());
    daemon
        .apply_feedback(
            session,
            NetworkFeedback {
                acknowledged_bitrate_bits_per_second: 500_000,
                round_trip_time_us: 400_000,
                lost_packets_per_mille: 100,
            },
        )
        .unwrap();
    assert!(daemon.should_force_keyframe(session).unwrap());
    daemon.close(session).unwrap();
    assert_eq!(daemon.active_sessions(), 0);
}

#[test]
fn packetizer_rejects_bad_mtu_and_ssrc() {
    assert!(matches!(
        Av1RtpPacketizer::new(13, 98),
        Err(ghostos_remote_display::RtpPacketError::InvalidMtu)
    ));
    let packetizer = Av1RtpPacketizer::new(100, 98).unwrap();
    let frame = EncodedFrame {
        frame_id: 1,
        timestamp_90khz: 1,
        buffer: buffer(1),
        bytes: 1,
        keyframe: false,
        temporal_id: 0,
    };
    let mut output = [ghostos_remote_display::RtpPacket {
        payload_type: 0,
        sequence: 0,
        timestamp: 0,
        ssrc: 0,
        marker: false,
        av1_aggregation_header: 0,
        payload: buffer(1),
    }; 1];
    let mut sequence = 0;
    assert_eq!(packetizer.packetize(frame, 0, &mut sequence, &mut output), Err(ghostos_remote_display::RtpPacketError::InvalidSsrc));
}
