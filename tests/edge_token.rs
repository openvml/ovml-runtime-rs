#[test]
fn edge_sec_ms_gec_matches_reference_v2_tokens() {
    assert_eq!(
        kothok_edge_tts::sec_ms_gec_at(1_700_000_000.0),
        "2CD23A2CA0B2595A-F3C846C370E05F9CC83ACC5F995C5F5D87A4D0F14CE8936A0F902E64099E38A2"
    );
    assert_eq!(
        kothok_edge_tts::sec_ms_gec_at(1_794_018_579.0),
        "4D31DCFA29C9AA65-794B6417CFCF59B156B0D567FA3DE790C76AABE8EFFC044E38991D5E546C744B"
    );
}
