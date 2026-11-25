use super::*;

/// Runs both peers on one virtual clock and an in-memory link, so the result
/// depends on neither wall time, scheduler load nor the UDP stack.
fn pump(
    from: &mut Rtc,
    to: &mut Rtc,
    now: Instant,
    events: &mut Vec<Event>,
) -> Result<(bool, Instant)> {
    let mut moved = false;
    loop {
        match from.poll_output()? {
            Output::Transmit(packet) => {
                let data = Receive::new(
                    Protocol::Udp,
                    packet.source,
                    packet.destination,
                    &packet.contents,
                )?;
                to.handle_input(Input::Receive(now, data))?;
                moved = true;
            }
            Output::Event(event) => events.push(event),
            Output::Timeout(timeout) => return Ok((moved, timeout)),
        }
    }
}

#[tokio::test]
async fn negotiated_audio_track_carries_speech_samples() -> Result<()> {
    let addresses = [std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)];
    let mut client = Peer::bind(addresses)?;
    let mut server = Peer::bind(addresses)?;
    let (offer, pending, client_mid) = client.offer()?;
    let offer = str0m::change::SdpOffer::from_sdp_string(&offer)?;
    let answer = server.rtc.sdp_api().accept_offer(offer)?.to_sdp_string();
    client.accept_answer(pending, &answer)?;

    let frame: [f32; FRAME_SAMPLES] = std::array::from_fn(|i| (i as f32 * 0.07).sin() * 0.5);
    let mut encoder = VoiceEncoder::new()?;
    let mut decoder = opus::Decoder::new(SAMPLE_RATE, opus::Channels::Mono)?;
    let mut now = Instant::now();
    let (mut client_events, mut server_events) = (Vec::new(), Vec::new());
    let (mut client_ready, mut server_ready, mut sent) = (false, false, false);

    // Bounded by virtual steps, not seconds: a handshake needs a few hundred.
    for _ in 0..100_000 {
        if client_ready && server_ready && !sent {
            let writer = client.rtc.writer(client_mid).context("no audio track")?;
            let params = writer.payload_params().next().context("no Opus")?;
            let (pt, frequency) = (params.pt(), params.spec().clock_rate);
            let len = encoder.opus.encode_float(&frame, &mut encoder.packet)?;
            writer.write(
                pt,
                now,
                MediaTime::new(0, frequency),
                &encoder.packet[..len],
            )?;
            sent = true;
        }
        let (moved_c, timeout_c) = pump(&mut client.rtc, &mut server.rtc, now, &mut client_events)?;
        let (moved_s, timeout_s) = pump(&mut server.rtc, &mut client.rtc, now, &mut server_events)?;
        client_ready |= client_events.iter().any(|e| matches!(e, Event::Connected));
        server_ready |= server_events.iter().any(|e| matches!(e, Event::Connected));
        for event in server_events.drain(..) {
            if let Event::MediaData(data) = event {
                let mut samples = [0.; FRAME_SAMPLES];
                let count = decoder.decode_float(&data.data, &mut samples, false)?;
                assert_eq!(count, FRAME_SAMPLES);
                assert!(samples.iter().any(|sample| sample.abs() > 0.05));
                return Ok(());
            }
        }
        if !(moved_c || moved_s) {
            now = now.max(timeout_c.min(timeout_s));
            client.rtc.handle_input(Input::Timeout(now))?;
            server.rtc.handle_input(Input::Timeout(now))?;
        }
    }
    anyhow::bail!("voice handshake did not finish within the virtual step budget")
}
