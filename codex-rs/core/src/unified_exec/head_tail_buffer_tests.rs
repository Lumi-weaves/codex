use super::HeadTailBuffer;

use pretty_assertions::assert_eq;

#[test]
fn keeps_prefix_and_suffix_when_over_budget() {
    let mut buf = HeadTailBuffer::new(/*max_bytes*/ 10);

    buf.push_chunk(b"0123456789".to_vec());
    assert_eq!(buf.omitted_bytes(), 0);

    // Exceeds max by 2; we should keep head+tail and omit the middle.
    buf.push_chunk(b"ab".to_vec());
    assert!(buf.omitted_bytes() > 0);

    let rendered = String::from_utf8_lossy(&buf.to_bytes()).to_string();
    assert!(rendered.starts_with("01234"));
    assert!(rendered.ends_with("89ab"));
    assert_eq!(
        String::from_utf8_lossy(&buf.to_bytes_with_omission_marker()),
        "01234\n... 2 bytes omitted ...\n789ab"
    );
}

#[test]
fn max_bytes_zero_drops_everything() {
    let mut buf = HeadTailBuffer::new(/*max_bytes*/ 0);
    buf.push_chunk(b"abc".to_vec());

    assert_eq!(buf.retained_bytes(), 0);
    assert_eq!(buf.omitted_bytes(), 3);
    assert_eq!(buf.to_bytes(), b"".to_vec());
    assert_eq!(buf.snapshot_chunks(), Vec::<Vec<u8>>::new());
}

#[test]
fn head_budget_zero_keeps_only_last_byte_in_tail() {
    let mut buf = HeadTailBuffer::new(/*max_bytes*/ 1);
    buf.push_chunk(b"abc".to_vec());

    assert_eq!(buf.retained_bytes(), 1);
    assert_eq!(buf.omitted_bytes(), 2);
    assert_eq!(buf.to_bytes(), b"c".to_vec());
}

#[test]
fn draining_resets_state_and_push_buffer_preserves_omissions() {
    let mut buf = HeadTailBuffer::new(/*max_bytes*/ 10);
    buf.push_chunk(b"0123456789".to_vec());
    buf.push_chunk(b"ab".to_vec());

    let drained = buf.drain();
    let mut collected = HeadTailBuffer::new(/*max_bytes*/ 10);
    collected.push_buffer(drained);

    assert_eq!(buf.retained_bytes(), 0);
    assert_eq!(buf.omitted_bytes(), 0);
    assert_eq!(buf.to_bytes(), b"".to_vec());
    assert_eq!(collected.to_bytes(), b"01234789ab".to_vec());
    assert_eq!(collected.omitted_bytes(), 2);
    assert_eq!(collected.total_bytes(), 12);
}

#[test]
fn chunk_larger_than_tail_budget_keeps_only_tail_end() {
    let mut buf = HeadTailBuffer::new(/*max_bytes*/ 10);
    buf.push_chunk(b"0123456789".to_vec());

    // Tail budget is 5 bytes. This chunk should replace the tail and keep only its last 5 bytes.
    buf.push_chunk(b"ABCDEFGHIJK".to_vec());

    let out = String::from_utf8_lossy(&buf.to_bytes()).to_string();
    assert!(out.starts_with("01234"));
    assert!(out.ends_with("GHIJK"));
    assert!(buf.omitted_bytes() > 0);
}

#[test]
fn fills_head_then_tail_across_multiple_chunks() {
    let mut buf = HeadTailBuffer::new(/*max_bytes*/ 10);

    // Fill the 5-byte head budget across multiple chunks.
    buf.push_chunk(b"01".to_vec());
    buf.push_chunk(b"234".to_vec());
    assert_eq!(buf.to_bytes(), b"01234".to_vec());

    // Then fill the 5-byte tail budget.
    buf.push_chunk(b"567".to_vec());
    buf.push_chunk(b"89".to_vec());
    assert_eq!(buf.to_bytes(), b"0123456789".to_vec());
    assert_eq!(buf.omitted_bytes(), 0);

    // One more byte causes the tail to drop its oldest byte.
    buf.push_chunk(b"a".to_vec());
    assert_eq!(buf.to_bytes(), b"012346789a".to_vec());
    assert_eq!(buf.omitted_bytes(), 1);
}

#[test]
fn empty_and_tiny_chunks_have_bounded_metadata() {
    let mut buf = HeadTailBuffer::new(/*max_bytes*/ 10);

    for byte in b"0123456789ab" {
        buf.push_chunk(Vec::new());
        buf.push_chunk(vec![*byte]);
    }

    assert_eq!(
        buf.snapshot_chunks(),
        vec![b"01234".to_vec(), b"789ab".to_vec()]
    );
    assert_eq!(buf.retained_bytes(), 10);
    assert_eq!(buf.omitted_bytes(), 2);
}

#[test]
fn bounded_excerpt_keeps_true_prefix_and_recent_suffix_and_reports_omitted() {
    let mut buf = HeadTailBuffer::new(/*max_bytes*/ 100);
    for byte in b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ" {
        buf.push_chunk(vec![*byte]);
    }
    // 62 bytes retained (head 50, tail 12), nothing omitted yet.
    assert_eq!(buf.retained_bytes(), 62);
    assert_eq!(buf.omitted_bytes(), 0);

    // Marker must be able to report the worst-case omitted count, so the
    // content budget is max_bytes minus the marker ceiling: 40 - 26 = 14
    // (head 7, tail 7).
    let (excerpt, omitted) = buf.bounded_excerpt(/*max_bytes*/ 40);
    assert_eq!(omitted, 62 - 14);
    assert!(
        excerpt.len() <= 40,
        "excerpt must stay bounded: {excerpt:?}"
    );
    // The true head prefix and the true most-recent tail suffix survive.
    assert!(excerpt.starts_with(b"01234".as_slice()));
    assert!(excerpt.ends_with(b"YZ".as_slice()));
    let excerpt_text = String::from_utf8_lossy(&excerpt);
    assert!(
        excerpt_text.contains("bytes omitted"),
        "excerpt trimming must include one omission marker"
    );
}

#[test]
fn bounded_excerpt_keeps_omission_marker_and_does_not_double_count() {
    let mut buf = HeadTailBuffer::new(/*max_bytes*/ 10);
    for byte in b"0123456789abcdef" {
        buf.push_chunk(vec![*byte]);
    }
    // 10 retained (head 5 + tail 5), 6 omitted by the buffer.
    assert_eq!(buf.retained_bytes(), 10);
    assert_eq!(buf.omitted_bytes(), 6);

    let (excerpt, total_omitted) = buf.bounded_excerpt(/*max_bytes*/ 64);
    assert!(
        excerpt.len() <= 64,
        "excerpt must stay bounded: {excerpt:?}"
    );
    // The total omitted count is exactly the transcript's own omissions
    // (nothing more was trimmed because the whole retained sequence fits in
    // the content budget), and the excerpt keeps all retained content with
    // one bounded marker.
    assert_eq!(total_omitted, 6);
    assert_eq!(
        excerpt,
        b"01234bcdef\n... 6 bytes omitted ...\n".to_vec(),
        "all retained logical bytes plus one omission marker"
    );
}

#[test]
fn bounded_excerpt_keeps_tail_suffix_when_trimming() {
    let mut buf = HeadTailBuffer::new(/*max_bytes*/ 100);
    for byte in b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ" {
        buf.push_chunk(vec![*byte]);
    }
    // 62 retained (head 50, tail 12 = "OPQRSTUVWXYZ"), no omissions.
    assert_eq!(buf.retained_bytes(), 62);
    assert_eq!(buf.omitted_bytes(), 0);

    // Cap 40: marker ceiling 26 leaves a content budget of 14 (head 7,
    // tail 7), so both the head prefix and the tail are trimmed and the tail
    // keeps its suffix.
    let (excerpt, total_omitted) = buf.bounded_excerpt(/*max_bytes*/ 40);
    assert_eq!(total_omitted, 62 - 14);
    assert_eq!(excerpt.len(), 40);
    // The tail keeps its SUFFIX (the most recent output), not its prefix.
    assert!(excerpt.ends_with(b"TUVWXYZ".as_slice()));
    let excerpt_text = String::from_utf8_lossy(&excerpt);
    assert!(
        !excerpt_text.contains("OPQRST"),
        "tail prefix must be dropped"
    );
}

#[test]
fn bounded_excerpt_total_omitted_combines_transcript_and_excerpt_omissions() {
    let mut buf = HeadTailBuffer::new(/*max_bytes*/ 10);
    for byte in b"0123456789abcdefghijklmnopqrstuvwxyz" {
        buf.push_chunk(vec![*byte]);
    }
    // 10 retained (head 5 + tail 5), 26 omitted by the buffer.
    assert_eq!(buf.retained_bytes(), 10);
    assert_eq!(buf.omitted_bytes(), 26);

    // Marker ceiling for total 36: 26 bytes; content budget 30 - 26 = 4
    // (head 2, tail 2): 6 retained bytes are additionally trimmed, so the
    // total omitted is 26 + 6 = 32.
    let (excerpt, total_omitted) = buf.bounded_excerpt(/*max_bytes*/ 30);
    assert_eq!(total_omitted, 32);
    assert!(excerpt.len() <= 30);
    assert!(excerpt.ends_with(b"yz".as_slice()));
}

#[test]
fn bounded_excerpt_of_large_transcript_keeps_head_and_tail() {
    // Production-shaped: the default 1 MiB store cap means a 4 KiB transcript
    // lives entirely in the internal head half. The excerpt must still select
    // from the logical retained byte sequence, keeping the true most-recent
    // suffix rather than only a prefix.
    let mut buf = HeadTailBuffer::default();
    buf.push_chunk((0..4096u16).map(|i| (i % 251) as u8).collect());
    assert_eq!(buf.retained_bytes(), 4096);
    assert_eq!(buf.omitted_bytes(), 0);
    assert_eq!(
        buf.snapshot_chunks().len(),
        1,
        "all bytes live in the head half"
    );

    let (excerpt, omitted) = buf.bounded_excerpt(/*max_bytes*/ 2048);
    // Marker ceiling for total 4096 is 28 bytes ("\n... 4096 bytes omitted
    // ...\n"); content budget 2048 - 28 = 2020 (head 1010, tail 1010).
    assert_eq!(excerpt.len(), 2048);
    assert_eq!(omitted, 4096 - 2020);
    let retained = buf.to_bytes();
    assert_eq!(
        &excerpt[..1010],
        &retained[..1010],
        "excerpt prefix is the true logical prefix"
    );
    assert_eq!(
        &excerpt[excerpt.len() - 1010..],
        &retained[retained.len() - 1010..],
        "excerpt suffix is the true logical most-recent suffix"
    );
    assert_eq!(
        *excerpt.last().expect("excerpt non-empty"),
        *retained.last().expect("buffer non-empty"),
        "excerpt must end with the transcript's actual last byte"
    );
    let excerpt_text = String::from_utf8_lossy(&excerpt);
    assert!(excerpt_text.contains("bytes omitted"));
}

#[test]
fn bounded_excerpt_saturates_omission_accounting_on_overflow() {
    let mut buf = HeadTailBuffer::new(usize::MAX);
    // Push enough to force omissions without overflowing any counters.
    buf.push_chunk(vec![b'a'; 1024]);
    buf.push_chunk(vec![b'b'; 1024]);
    assert_eq!(buf.omitted_bytes(), 0);
    assert_eq!(buf.retained_bytes(), 2048);

    let (excerpt, total_omitted) = buf.bounded_excerpt(usize::MAX);
    assert_eq!(excerpt.len(), 2048);
    assert_eq!(total_omitted, 0);

    // A zero cap reports every observed byte as omitted without overflow.
    let (empty, total_omitted) = buf.bounded_excerpt(/*max_bytes*/ 0);
    assert!(empty.is_empty());
    assert_eq!(total_omitted, buf.total_bytes());
}

#[test]
fn bounded_excerpt_reports_exact_omissions_for_invalid_bytes() {
    let mut buf = HeadTailBuffer::new(/*max_bytes*/ 10);
    buf.push_chunk(vec![0xff; 6]); // head 5 + 1 tail byte
    buf.push_chunk(vec![b'a'; 8]); // tail keeps its last 5 bytes
    // Retained 10, omitted 4 (one 0xff and three 'a' bytes dropped by the
    // transcript itself).
    assert_eq!(buf.retained_bytes(), 10);
    assert_eq!(buf.omitted_bytes(), 4);
    assert_eq!(buf.total_bytes(), 14);

    // Marker ceiling for total 14 is 26 bytes; content budget 30 - 26 = 4
    // (head 2, tail 2). Omitted = 14 - 4 = 10, exact even though the head
    // content is malformed bytes.
    let (excerpt, omitted) = buf.bounded_excerpt(/*max_bytes*/ 30);
    assert_eq!(omitted, 10);
    assert!(excerpt.len() <= 30);
    assert_eq!(&excerpt[..2], &[0xff, 0xff], "head prefix preserved");
    assert_eq!(
        &excerpt[excerpt.len() - 2..],
        b"aa",
        "most-recent tail suffix preserved"
    );
}

#[test]
fn bounded_excerpt_is_identity_when_within_cap() {
    let mut buf = HeadTailBuffer::new(/*max_bytes*/ 100);
    buf.push_chunk(b"hello world".to_vec());
    let (excerpt, extra_omitted) = buf.bounded_excerpt(/*max_bytes*/ 100);
    assert_eq!(extra_omitted, 0);
    assert_eq!(excerpt, b"hello world".to_vec());
}
