use core::fmt;
use ffi;
use ffi::RANGEPROOF_MAX_LENGTH;
use std::ops::Range;
use Error;
use Generator;
use PedersenCommitment;
use Verification;
use {Secp256k1, SecretKey, Signing};

/// Represents a range proof.
///
/// TODO: Store rangeproof info
#[derive(Debug, PartialEq, Clone, Eq, Hash)]
pub struct RangeProof {
    inner: ffi::RangeProof,
}

impl RangeProof {
    /// Serialize to bytes.
    pub fn serialize(&self) -> Vec<u8> {
        self.inner.to_bytes()
    }

    /// Parse from byte slice.
    ///
    /// TODO: Rename to parse (and other similar functions)
    pub fn from_slice(bytes: &[u8]) -> Result<Self, Error> {
        let mut exp = 0;
        let mut mantissa = 0;
        let mut min_value = 0;
        let mut max_value = 0;

        let ret = unsafe {
            ffi::secp256k1_rangeproof_info(
                ffi::secp256k1_context_no_precomp,
                &mut exp,
                &mut mantissa,
                &mut min_value,
                &mut max_value,
                bytes.as_ptr(),
                bytes.len(),
            )
        };

        if ret == 0 {
            return Err(Error::InvalidRangeProof);
        }

        Ok(RangeProof {
            inner: ffi::RangeProof::new(bytes),
        })
    }

    /// Get length.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Check if it's empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

#[cfg(feature = "bitcoin_hashes")]
impl fmt::Display for RangeProof {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use bitcoin_hashes::hex::format_hex;

        format_hex(self.serialize().as_slice(), f)
    }
}

// TODO: Macrofy (de)serialization

#[cfg(all(feature = "serde", feature = "bitcoin_hashes"))]
impl ::serde::Serialize for RangeProof {
    fn serialize<S: ::serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if s.is_human_readable() {
            s.collect_str(&self)
        } else {
            s.serialize_bytes(&self.serialize())
        }
    }
}

#[cfg(all(feature = "serde", feature = "bitcoin_hashes"))]
impl<'de> ::serde::Deserialize<'de> for RangeProof {
    fn deserialize<D: ::serde::Deserializer<'de>>(d: D) -> Result<RangeProof, D::Error> {
        if d.is_human_readable() {
            struct HexVisitor;

            impl<'de> ::serde::de::Visitor<'de> for HexVisitor {
                type Value = RangeProof;

                fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                    formatter.write_str("an ASCII hex string")
                }

                fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
                where
                    E: ::serde::de::Error,
                {
                    use bitcoin_hashes::hex::FromHex;

                    let bytes = Vec::<u8>::from_hex(v).map_err(E::custom)?;
                    RangeProof::from_slice(&bytes).map_err(E::custom)
                }
            }
            d.deserialize_str(HexVisitor)
        } else {
            struct BytesVisitor;

            impl<'de> ::serde::de::Visitor<'de> for BytesVisitor {
                type Value = RangeProof;

                fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                    formatter.write_str("a bytestring")
                }

                fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
                where
                    E: ::serde::de::Error,
                {
                    RangeProof::from_slice(v).map_err(E::custom)
                }
            }

            d.deserialize_bytes(BytesVisitor)
        }
    }
}

// TODO: Could model PedersenCommitmentOpening separately
// TODO: Use generic `M` for message (so that we can use it with `RangeProofMessage`)
pub struct Opening {
    pub value: u64,
    pub blinding_factor: SecretKey,
    pub message: Box<[u8]>,
}

impl<C: Signing> Secp256k1<C> {
    /// Prove that `commitment` hides a value within a range, with the lower bound set to `min_value`.
    pub fn make_rangeproof(
        &self,
        min_value: u64,
        commitment: PedersenCommitment,
        value: u64,
        commitment_blinding: SecretKey,
        message: &[u8],
        additional_commitment: &[u8],
        sk: SecretKey,
        exp: i32,
        min_bits: u8,
        additional_generator: Generator,
    ) -> Result<RangeProof, Error> {
        let mut proof = [0u8; RANGEPROOF_MAX_LENGTH];
        let mut proof_length = RANGEPROOF_MAX_LENGTH;

        let ret = unsafe {
            ffi::secp256k1_rangeproof_sign(
                self.ctx,
                proof.as_mut_ptr(),
                &mut proof_length,
                min_value,
                commitment.as_inner(),
                commitment_blinding.as_ptr(),
                sk.as_ptr(),
                exp,
                min_bits as i32,
                value,
                message.as_ptr(),
                message.len(),
                additional_commitment.as_ptr(),
                additional_commitment.len(),
                additional_generator.as_inner(),
            )
        };

        if ret == 0 {
            return Err(Error::CannotMakeRangeProof);
        }

        Ok(RangeProof {
            inner: ffi::RangeProof::new(&proof[..proof_length]),
        })
    }
}

impl<C: Verification> Secp256k1<C> {
    /// Verify that the committed value is within a range.
    ///
    /// If the verification is successful, return the actual range of possible values.
    pub fn verify_rangeproof(
        &self,
        proof: RangeProof,
        commitment: PedersenCommitment,
        additional_commitment: &[u8],
        additional_generator: Generator,
    ) -> Result<Range<u64>, Error> {
        let mut min_value = 0u64;
        let mut max_value = 0u64;

        let ret = unsafe {
            ffi::secp256k1_rangeproof_verify(
                self.ctx,
                &mut min_value,
                &mut max_value,
                commitment.as_inner(),
                proof.inner.as_ptr(),
                proof.inner.len(),
                additional_commitment.as_ptr(),
                additional_commitment.len(),
                additional_generator.as_inner(),
            )
        };

        if ret == 0 {
            return Err(Error::InvalidRangeProof);
        }

        Ok(Range {
            start: min_value,
            end: max_value + 1,
        })
    }

    /// Verify a range proof proof and rewind the proof to recover information sent by its author.
    pub fn rewind_rangeproof(
        &self,
        proof: &RangeProof,
        commitment: PedersenCommitment,
        sk: SecretKey,
        additional_commitment: &[u8],
        additional_generator: Generator,
    ) -> Result<(Opening, Range<u64>), Error> {
        let mut min_value = 0u64;
        let mut max_value = 0u64;

        let mut blinding_factor = [0u8; 32];
        let mut value = 0u64;
        let mut message = [0u8; 4096];
        let mut message_length = 4096usize;

        let ret = unsafe {
            ffi::secp256k1_rangeproof_rewind(
                self.ctx,
                blinding_factor.as_mut_ptr(),
                &mut value,
                message.as_mut_ptr(),
                &mut message_length,
                sk.as_ptr(),
                &mut min_value,
                &mut max_value,
                commitment.as_inner(),
                proof.inner.as_ptr(),
                proof.inner.len(),
                additional_commitment.as_ptr(),
                additional_commitment.len(),
                additional_generator.as_inner(),
            )
        };

        if ret == 0 {
            return Err(Error::InvalidRangeProof);
        }

        let opening = Opening {
            value,
            blinding_factor: SecretKey::from_slice(&blinding_factor)?,
            message: message[..message_length].into(),
        };

        let range = Range {
            start: min_value,
            end: max_value + 1,
        };

        Ok((opening, range))
    }
}

#[cfg(all(test, feature = "global-context"))] // use global context for convenience
mod tests {
    use super::*;
    use rand::thread_rng;
    use SECP256K1;
    use {CommitmentSecrets, Tag};

    #[test]
    fn create_and_verify_range_proof() {
        let value = 1_000;
        let commitment_secrets = CommitmentSecrets::random(value);
        let tag = Tag::random();
        let commitment = commitment_secrets.commit(tag);

        let message = b"foo";
        let additional_commitment = b"bar";

        let sk = SecretKey::new(&mut thread_rng());
        let additional_generator =
            SECP256K1.blind(tag, commitment_secrets.generator_blinding_factor);

        let proof = SECP256K1
            .make_rangeproof(
                1,
                commitment,
                value,
                commitment_secrets.value_blinding_factor,
                message,
                additional_commitment,
                sk,
                0,
                52,
                additional_generator,
            )
            .unwrap();

        SECP256K1
            .verify_rangeproof(
                proof,
                commitment,
                additional_commitment,
                additional_generator,
            )
            .unwrap();
    }

    #[test]
    fn rewind_range_proof() {
        let value = 1_000;
        let commitment_secrets = CommitmentSecrets::random(value);
        let tag = Tag::random();
        let commitment = commitment_secrets.commit(tag);

        let message = b"foo";
        let additional_commitment = b"bar";

        let sk = SecretKey::new(&mut thread_rng());
        let additional_generator =
            SECP256K1.blind(tag, commitment_secrets.generator_blinding_factor);

        let proof = SECP256K1
            .make_rangeproof(
                1,
                commitment,
                value,
                commitment_secrets.value_blinding_factor,
                message,
                additional_commitment,
                sk,
                0,
                52,
                additional_generator,
            )
            .unwrap();

        let (opening, _range) = SECP256K1
            .rewind_rangeproof(
                &proof,
                commitment,
                sk,
                additional_commitment,
                additional_generator,
            )
            .unwrap();

        assert_eq!(opening.value, commitment_secrets.value);
        assert_eq!(
            opening.blinding_factor,
            commitment_secrets.value_blinding_factor
        );

        // TODO: File bug with upstream: message length is not set correctly
        assert!(opening.message.starts_with(message));
    }
}
