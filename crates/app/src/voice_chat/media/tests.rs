use super::*;

#[tokio::test]
async fn negotiated_audio_track_carries_speech_samples() -> Result<()> {
    let addresses = [std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)];
    let mut client = Peer::bind(addresses)?;
    let mut server = Peer::bind(addresses)?;
    let (offer, pending, client_mid) = client.offer()?;
    let offer = str0m::change::SdpOffer::from_sdp_string(&offer)?;
    let answer = server.rtc.sdp_api().accept_offer(offer)?.to_sdp_string();
    client.accept_answer(pending, &answer)?;

    let frame = std::array::from_fn(|i| (i as f32 * 0.07).sin() * 0.5);
    let mut encoder = VoiceEncoder::new()?;
    let mut decoder = opus::Decoder::new(SAMPLE_RATE, opus::Channels::Mono)?;
    let mut client_ready = false;
    let mut server_ready = false;
    let mut sent = false;
    // Hard upper limit for runners without nextest; far above any loaded-host handshake.
    let deadline = tokio::time::sleep(Duration::from_secs(120));
    tokio::pin!(deadline);
    loop {
        if client_ready && server_ready && !sent {
            client.write(client_mid, &mut encoder, &frame)?;
            sent = true;
        }
        tokio::select! {
            () = &mut deadline => anyhow::bail!("voice handshake did not finish in 120 s"),
            event = client.next_event() => {
                client_ready |= matches!(event?, Event::Connected);
            }
            event = server.next_event() => match event? {
                Event::Connected => server_ready = true,
                Event::MediaData(data) => {
                    let mut samples = [0.; FRAME_SAMPLES];
                    let count = decoder.decode_float(&data.data, &mut samples, false)?;
                    assert_eq!(count, FRAME_SAMPLES);
                    assert!(samples.iter().any(|sample| sample.abs() > 0.05));
                    return Ok(());
                }
                _ => {}
            }
        }
    }
}
