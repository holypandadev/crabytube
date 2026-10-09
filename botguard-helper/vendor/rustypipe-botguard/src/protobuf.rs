fn parse_varint<P: Iterator<Item = u8>>(pb: &mut P) -> Option<u64> {
    let mut result = 0;
    let mut num_read = 0;

    for b in pb.by_ref() {
        let value = b & 0x7f;
        result |= u64::from(value) << (7 * num_read);
        num_read += 1;

        if b & 0x80 == 0 {
            break;
        }
    }
    if num_read == 0 {
        None
    } else {
        Some(result)
    }
}

fn parse_field<P: Iterator<Item = u8>>(pb: &mut P) -> Option<(u32, u8)> {
    parse_varint(pb).map(|v| {
        let f = (v >> 3) as u32;
        let w = (v & 0x07) as u8;
        (f, w)
    })
}

pub fn bytes_from_pb<P: IntoIterator<Item = u8>>(pb: P, field: u32) -> Option<Vec<u8>> {
    let mut pb = pb.into_iter();
    while let Some((this_field, wire)) = parse_field(&mut pb) {
        let to_skip = match wire {
            // varint
            0 => {
                parse_varint(&mut pb);
                0
            }
            // fixed 64bit
            1 => 8,
            // fixed 32bit
            5 => 4,
            // string
            2 => {
                let len = parse_varint(&mut pb)?;
                if this_field == field {
                    let mut buf = Vec::new();
                    for _ in 0..len {
                        buf.push(pb.next()?);
                    }
                    return Some(buf);
                }
                len
            }
            _ => return None,
        };
        for _ in 0..to_skip {
            pb.next();
        }
    }
    None
}

mod tests {
    #[test]
    fn parse_proto() {
        let p = b"MlJ0C4bw9OdQ1nQ3M4AKb9hdTYvhORYpaKAnKK5fE1Jcf5kd988G3WHrMWU-nCS0nDwoHlg27bfcBHpSg0_F3aaXu3da8V2QE3q0M_97L-iBTWF0";
        let p_bytes = data_encoding::BASE64URL.decode(p).unwrap();
        let res = super::bytes_from_pb(p_bytes, 6).unwrap();
        assert_eq!(data_encoding::BASE64URL.encode(&res), "dAuG8PTnUNZ0NzOACm_YXU2L4TkWKWigJyiuXxNSXH-ZHffPBt1h6zFlPpwktJw8KB5YNu233AR6UoNPxd2ml7t3WvFdkBN6tDP_ey_ogU1hdA==");
    }
}
