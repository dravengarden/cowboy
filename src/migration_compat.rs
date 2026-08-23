//! Compatibility boundary for the one-time migration-history squash.
//!
//! A database created by the preceding release has the complete legacy ledger.
//! Before the consolidated baseline migrator runs, Cowboy verifies every legacy
//! checksum and records the new baseline as already applied. Fresh databases run
//! the baseline and receive the legacy ledger rows from the SQL itself, keeping
//! an immediate binary rollback safe in both directions.

use std::fmt::Write as _;

use anyhow::{Result, ensure};

pub(crate) const POSTGRES_BASELINE_VERSION: i64 = 34;
pub(crate) const SQLITE_BASELINE_VERSION: i64 = 8;

pub(crate) const LEGACY_POSTGRES_MIGRATIONS: &[(i64, &str)] = &[
    (
        1,
        "6b8c96bfaa20be52c2cc27d985c0c5409533b375a4a9443d8fb9aef0f86fb7b5cacb97b2f504006cd23c76cdf820193f",
    ),
    (
        2,
        "ad82ac0a149e30e4510cea7841ca731a80fb1cda357a65a5a2649edd46d7bf703d93fd74af333c0356a48a0e3540b453",
    ),
    (
        3,
        "2d72276733684ccf051498bf4d837dfa0227ff58fe60d6a65814e7417b269090cb84194821025c3b8ca1822932c50c3e",
    ),
    (
        4,
        "9719a53de1f71840487fc264672aa54f9e3738c94a5b5f370795d4a0941621e1c818b2e393e089371622e7555c1d182c",
    ),
    (
        5,
        "32f2518e8b36acc3657511cefae4eb3e188f0b5af162eb649e2de5684e1d2be141df4e3974966811ab168256e97f1c17",
    ),
    (
        6,
        "c48955b47d0d98fe31a67c88f576bf0ff413e758882c694b833f7bc7d0421b8de6a143099177b4f73e56c6978f1bdc02",
    ),
    (
        7,
        "17a031e99e34cc7dc7cfc34cee4ce4ccf7513733168d013fa442a450a1774f571b111f9b312780588b72b7b32c84e2e1",
    ),
    (
        8,
        "cffedc29c0204aa8b850ec933028ad5ae956298ea75197e30e1f7f776372c1c022b0a2ade8c8e5ac06b2604dacce269c",
    ),
    (
        9,
        "8414e8b198e54343f829d7c30a3a54fd3b6b9248d49959657ae6ae7f495382e9375373e346128bbfe785bc35184f5900",
    ),
    (
        10,
        "0a06f9bf0b90ed61dc3b51895af4c583d1c3cdaec9587115ce4960e1c97c1cb976f995c3ef076852ddab0f932dcc6992",
    ),
    (
        11,
        "27fa174e8727c73555744859fd9a7d5ca2da032bc7cd8170414ad6fbeddc900f162663019dc2e917bff35f301f8245be",
    ),
    (
        12,
        "43d8a811f10bdfc95c8a04ea22700da71c4e040e7bc4f2e01eb1368db963d2d45056c2f16af3016ea1f644b0a0b0744b",
    ),
    (
        13,
        "bb0861722480a3420fa57d7c1cd3fc11b21b5d3d0454f2cd248b87cdde08c0af9c9a6da86fcc89132b478f85d63fa79e",
    ),
    (
        14,
        "4c052d863e01b316a0efd2b3a213ca13b0ddbc5b0a5e1d6958f57b9042c6edffea9cab18f8bd454717060cb6ebfd82a4",
    ),
    (
        15,
        "1dccce5e4bd651d8a038a9f1d0eed5cbc7b0a179569e1549a261936b9fff3c94a91b4ff86c933c886c7ac3ed1ca08c93",
    ),
    (
        16,
        "4a4e5fd713578d2e4e9aabad317c636b2960cf146463144c2ac3db3b2867ddcb8b90f13aa485ce7018f40cdc86bbf260",
    ),
    (
        17,
        "6c7058962c632b1d781053271ac4f2698d96a1679c6b25f3b64a54580525ff2f4de7fcd2a4d5570d907bb19d608ff590",
    ),
    (
        18,
        "4a7e4af23cb326e80f0fe3537d7ba869301a2c173711429ec848375a060eb374facd21971cd270fbe228c60ae3e1c2c1",
    ),
    (
        19,
        "ffa7ba7597d169f2ac2c7232393b765f598c144e011e6e6f32c4972ea809e21ea0373749fbccd327c1bc22e4abf28130",
    ),
    (
        20,
        "882faa5ac6d9b85380ced22d833874d901a00fe5b3e30e900728187196e9308d520923fb1bc8661c8b27b0ad51ad7245",
    ),
    (
        21,
        "8d1a9c226c7a47a4b8e0969459a8fc04b25fec29da04409947c15a6e42f31c4a12e02e8bdcec92eb3e7d01ae8298caf7",
    ),
    (
        22,
        "8ec1498e475d85235ac69485337156dc691cd983d74ff4d545143f2f5fe3bc64e237998c9c344a0fc15cd128faeea4fb",
    ),
    (
        23,
        "47c9c01db17a1ed2d555e4e158a396c52aa6995de62ff6d2e2395217514fba51e1688bee7c8e3825f0d5bcca496d8233",
    ),
    (
        24,
        "dcea269ba3b7febe962e372c65942d54d958cdae7cbe6e9fbbf335eb1c82267a6fcbc36cf6d13d1810ea896def82a663",
    ),
    (
        25,
        "3cc2b5faad457baa415ef117a2c57ff5298fe00d384e7e3ee777bd99405ea95d1e92b9959a758aff110d96556a521163",
    ),
    (
        26,
        "260511cce91f76be818ee39816c084710535501b01f6d8e38c5a2bb7f904c140d3ac807554bf052bb3b368f427e229e8",
    ),
    (
        27,
        "e5bc0cb900f35b0505d336bbe66855e4f25d9a7bd90c282cdfcb5c72d0666103a52fc82895aba1687ee3110d54949cdb",
    ),
    (
        28,
        "0000b3b61491566bbcfe844d61935ef11fab13841a535161d41e134aa8f98011e8085fc0311ceb6486dee7f34770dbbd",
    ),
    (
        29,
        "82c8276484865a2a62cb14088d7c3aea5a57fa992cf67ff35f89e753f1c9acdec14febe2f7303a4d4c12aff6f7db00c5",
    ),
    (
        30,
        "24e44a3eac807a8dfab114e852d2656c0e703d9e44b8c21ff6aa4557503b95df3d6a2b542b407678779ad75e604f391b",
    ),
    (
        31,
        "caaf278983d41a30d5673e519073a6a7131635396253f6f1157ad9613172b0bb9b4b01c88889c56ddcf057b1472e5bed",
    ),
    (
        32,
        "dbfe7732245c44427ba727d16231c3bc6bf8d3dcaee5fe2eca48d200d103131bf3ae6636fa3768acfa5b24078f15f8f8",
    ),
    (
        33,
        "cabf4ea80cf9891e1bff94e4a831b1b0a94ef7f37caa96b127c77c901cac4778c1de9fb1e471b6568121e1e42144b170",
    ),
];

pub(crate) const LEGACY_SQLITE_MIGRATIONS: &[(i64, &str)] = &[
    (
        1,
        "37d5d2723cf03d0a97d735b7077481b798c15c331f6d9619f8a2c5516067de6c2d5210d756395bb8a9b4b6c06585daa3",
    ),
    (
        2,
        "1998bf4bd51b478454c67b88f49c9898ee912c6b0e10c5c0bfc380039c70e2e93c9bf804f2e21ce6b927564c8e5e81ab",
    ),
    (
        3,
        "95a4bf0af4e3dd9191833437926b53aac86e1135679bd6861e0becea6ba0bdb908b8741953a24ed88ef1ff2599518c4e",
    ),
    (
        4,
        "6c3c72e4001e5c1c42cd06957a951f7eca965caaaa48b3bcda350498a8bc07d9f90f75348091884791f4972d99a1166d",
    ),
    (
        5,
        "e3cbea9ed87d7ac30f6526fccec2fba8213bba0916d504e7a9af4d141ca0b39519aa2f7980ad34a1c44aa8b33e3c8ea7",
    ),
    (
        6,
        "ea85426677156960acea20dd6b52d6d5a166bc2aa3a7a66bda848fef823c1253831b4f105480df42003ef4b5ce0faf1c",
    ),
    (
        7,
        "015e64f500138d31ae413fa5855efbef9c592ef27c9e5631ff0059193a1e3f3a2355befbb277088635235f9b7b0bfe75",
    ),
];

fn checksum_hex(checksum: &[u8]) -> String {
    let mut encoded = String::with_capacity(checksum.len() * 2);
    for byte in checksum {
        write!(encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

pub(crate) fn needs_baseline_marker(
    applied: &[(i64, bool, Vec<u8>)],
    baseline_version: i64,
    legacy: &[(i64, &str)],
) -> Result<bool> {
    if applied
        .iter()
        .any(|(version, _, _)| *version == baseline_version)
        || applied.is_empty()
    {
        return Ok(false);
    }
    ensure!(
        applied.len() == legacy.len(),
        "database has a partial or unknown migration history; expected {} legacy versions, found {}",
        legacy.len(),
        applied.len()
    );
    for (version, expected_checksum) in legacy {
        let (_, success, actual_checksum) = applied
            .iter()
            .find(|(candidate, _, _)| candidate == version)
            .ok_or_else(|| anyhow::anyhow!("database is missing legacy migration {version}"))?;
        ensure!(
            *success,
            "legacy migration {version} is marked unsuccessful"
        );
        ensure!(
            checksum_hex(actual_checksum) == *expected_checksum,
            "legacy migration {version} checksum does not match the published history"
        );
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_hex(value: &str) -> Vec<u8> {
        value
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                let text = std::str::from_utf8(pair).unwrap();
                u8::from_str_radix(text, 16).unwrap()
            })
            .collect()
    }

    #[test]
    fn only_a_complete_verified_legacy_ledger_needs_a_marker() {
        let complete = LEGACY_SQLITE_MIGRATIONS
            .iter()
            .map(|(version, checksum)| (*version, true, decode_hex(checksum)))
            .collect::<Vec<_>>();
        assert!(
            needs_baseline_marker(&complete, SQLITE_BASELINE_VERSION, LEGACY_SQLITE_MIGRATIONS)
                .unwrap()
        );
        assert!(
            !needs_baseline_marker(&[], SQLITE_BASELINE_VERSION, LEGACY_SQLITE_MIGRATIONS).unwrap()
        );

        let mut marked = complete.clone();
        marked.push((SQLITE_BASELINE_VERSION, true, vec![1]));
        assert!(
            !needs_baseline_marker(&marked, SQLITE_BASELINE_VERSION, LEGACY_SQLITE_MIGRATIONS)
                .unwrap()
        );
    }

    #[test]
    fn partial_or_modified_history_fails_closed() {
        let partial = vec![(1, true, decode_hex(LEGACY_SQLITE_MIGRATIONS[0].1))];
        assert!(
            needs_baseline_marker(&partial, SQLITE_BASELINE_VERSION, LEGACY_SQLITE_MIGRATIONS)
                .is_err()
        );

        let mut modified = LEGACY_SQLITE_MIGRATIONS
            .iter()
            .map(|(version, checksum)| (*version, true, decode_hex(checksum)))
            .collect::<Vec<_>>();
        modified[0].2[0] ^= 0xff;
        assert!(
            needs_baseline_marker(&modified, SQLITE_BASELINE_VERSION, LEGACY_SQLITE_MIGRATIONS)
                .is_err()
        );
    }
}
