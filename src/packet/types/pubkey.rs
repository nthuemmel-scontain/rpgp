use nom::IResult;
use enum_primitive::FromPrimitive;

use super::{Key, PrimaryKey, PublicKeyAlgorithm, KeyVersion};
use packet::{Tag, Packet};
use util::mpi;

named!(rsa_fields<(&[u8], &[u8])>, do_parse!(
    n: mpi >>
    e: mpi >>
    ((n, e))
));

named!(new_public_key_parser((&[u8], usize)) -> (u32, u16, PublicKeyAlgorithm, (&[u8], &[u8])), do_parse!(
       key_time: take_bits!(u32, 32)
    >>      alg: map_opt!(
                     take_bits!(u8, 8),
                     PublicKeyAlgorithm::from_u8
                 ) 
    >>   fields: bytes!(switch!(value!(&alg), 
                   &PublicKeyAlgorithm::RSA => call!(rsa_fields) |
                   &PublicKeyAlgorithm::RSAEncrypt => call!(rsa_fields) |
                   &PublicKeyAlgorithm::RSASign => call!(rsa_fields) 
                 ))
    >> ((key_time, 0, alg, fields))
));

named!(old_public_key_parser((&[u8], usize)) -> (u32, u16, PublicKeyAlgorithm, (&[u8], &[u8])), do_parse!(
       key_time: take_bits!(u32, 32)
    >>      exp: take_bits!(u16, 16) 
    >>      alg: map_opt!(
                     take_bits!(u8, 8),
                     PublicKeyAlgorithm::from_u8
                 )
    >> ((key_time, exp, alg, (&b""[..], &b""[..])))
));

named!(public_key_parser<PrimaryKey>, bits!(do_parse!(
          key_ver: map_opt!(
                       take_bits!(u8, 8), 
                       KeyVersion::from_u8
                   )
    >>    details: switch!(value!(&key_ver), 
                       &KeyVersion::V2 => call!(old_public_key_parser) |
                       &KeyVersion::V3 => call!(old_public_key_parser) |
                       &KeyVersion::V4 => call!(new_public_key_parser)
                   ) 
    >> (PrimaryKey::PublicKey{
        algorithm: details.2,
        version: key_ver,
        n: (details.3).0.to_vec(),
        e: (details.3).1.to_vec(),
    })
)));

fn take_sigs<'a>(packets: &'a Vec<Packet>, mut ctr: usize) -> Vec<&'a Packet> {
    let mut res = vec![];
    while ctr < packets.len() && packets[ctr].tag == Tag::Signature {
        res.push(&packets[ctr]);
        ctr += 1;
    }

    res
}


/// Parse a transferable public key
/// Ref: https://tools.ietf.org/html/rfc4880.html#section-11.1
pub fn parse<'a>(packets: Vec<Packet>) -> IResult<&'a [u8], Key> {
    let packets_len = packets.len();
    let mut ctr = 0;

    // -- One Public-Key packet
    // TODO: better error management
    assert_eq!(packets[ctr].tag, Tag::PublicKey);
    let (_, primary_key) = public_key_parser(packets[ctr].body.as_slice()).unwrap();
    ctr += 1;

    // -- Zero or more revocation signatures
    let rev_sigs = take_sigs(&packets, ctr);
    ctr += rev_sigs.len();

    // -- Zero or more User Attribute packets
    // -- Zero or more Subkey packets

    let mut user_ids = vec![];
    let mut user_attrs = vec![];

    while ctr < packets_len {
        match packets[ctr].tag {
            Tag::UserID => {
                let id = &packets[ctr];
                ctr += 1;

                // --- zero or more signature packets
                let sigs = take_sigs(&packets, ctr);
                ctr += sigs.len();

                user_ids.push((id, sigs));
            }
            Tag::UserAttribute => {
                let attr = &packets[ctr];
                ctr += 1;

                // --- zero or more signature packets
                let sigs = take_sigs(&packets, ctr);
                ctr += sigs.len();

                user_attrs.push((attr, sigs));
            }
            _ => break,
        }
    }

    let mut subkeys = vec![];
    while ctr < packets_len && packets[ctr].tag == Tag::PublicSubkey {
        // --- one Signature packet,
        // TODO: better error handling
        assert_eq!(packets[ctr + 1].tag, Tag::Signature, "Missing signature");

        let subkey = &packets[ctr];
        let sig = &packets[ctr + 1];
        ctr += 2;

        // --- optionally a revocation
        let rev = if packets_len > ctr && packets[ctr].tag == Tag::Signature {
            let sig = &packets[ctr];
            ctr += 1;
            // TODO: assert sig is revocation
            Some(sig)
        } else {
            None
        };

        subkeys.push((subkey, sig, rev));
    }

    // TODO: better error handling
    assert!(user_ids.len() > 0, "Missing user ids");

    // TODO: better error handling
    assert_eq!(ctr, packets_len);

    IResult::Done(&b""[..], Key { primary_key: primary_key })
}
