//! Build-time verification of constant-fold certificates.
//!
//! The analyzer's constant folder (`crates/stc-sema/src/consteval.rs`)
//! records a [`stc_hir::ConstCertificate`] for every value it produces through
//! the shared scalar kernel in [`crate::exact`]. Code generation calls
//! [`verify_certificates`] before projecting or emitting any IR, so a folded
//! value that reaches the emitted IR is tied to the kernel and to the
//! operands the folder recorded.
//!
//! The verifier deliberately lives outside [`crate::exact`]: `exact.rs` is
//! pinned as the Aeneas bridge source, and its bytes must stay identical to
//! the model provenance record in `lean/bridge-toolchain.toml`.

use stc_hir::{ConstCertOp, ConstCertificate};

use crate::exact::{self, IntType};

/// Verifies one constant-fold certificate by recomputing its operation.
///
/// The recomputation calls the same kernel functions that constant folding
/// calls, so a certificate can only pass when the folder and the kernel agree
/// on the recorded operands and result.
///
/// # Errors
///
/// Returns a message naming the operation and both results when the recorded
/// result is not the kernel's result, or when the kernel traps on the
/// recorded operands.
pub fn verify_certificate(certificate: &ConstCertificate) -> Result<(), String> {
    let source = IntType::new(certificate.source.0, certificate.source.1);
    let target = IntType::new(certificate.target.0, certificate.target.1);
    let recomputed = match certificate.op {
        ConstCertOp::Add => exact::add(source, certificate.left, certificate.right),
        ConstCertOp::Sub => exact::sub(source, certificate.left, certificate.right),
        ConstCertOp::Mul => exact::mul(source, certificate.left, certificate.right),
        ConstCertOp::Div => exact::div(source, certificate.left, certificate.right),
        ConstCertOp::Modulo => exact::modulo(source, certificate.left, certificate.right),
        ConstCertOp::Neg => exact::neg(source, certificate.left),
        ConstCertOp::BitAnd => {
            Ok(exact::bit_and(source, certificate.left as u128, certificate.right as u128) as i128)
        }
        ConstCertOp::BitOr => {
            Ok(exact::bit_or(source, certificate.left as u128, certificate.right as u128) as i128)
        }
        ConstCertOp::BitXor => {
            Ok(exact::bit_xor(source, certificate.left as u128, certificate.right as u128) as i128)
        }
        ConstCertOp::BitNot => Ok(exact::bit_not(source, certificate.left as u128) as i128),
        ConstCertOp::Convert => exact::convert(source, target, certificate.left),
    };
    match recomputed {
        Ok(value) if value == certificate.result => Ok(()),
        Ok(value) => Err(format!(
            "constant certificate for {:?} recorded {} but the exact kernel computes {}",
            certificate.op, certificate.result, value
        )),
        Err(trap) => Err(format!(
            "constant certificate for {:?} records a result but the exact kernel reports {trap:?}",
            certificate.op
        )),
    }
}

/// Verifies every certificate recorded by constant folding.
///
/// Code generation calls this before emitting IR. A folded value that reaches
/// the emitted IR therefore carries a recorded operation, its operands, and a
/// result that the shared kernel reproduces.
///
/// # Errors
///
/// Returns one message per mismatching certificate, in recording order.
pub fn verify_certificates(certificates: &[ConstCertificate]) -> Result<(), Vec<String>> {
    let failures: Vec<String> = certificates
        .iter()
        .filter_map(|certificate| verify_certificate(certificate).err())
        .collect();
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures)
    }
}
