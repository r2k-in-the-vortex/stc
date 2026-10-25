//! M5 phase 3: reference descriptors, dereference, and output negation.

use stc_hir::{DefId, ExprId, PlaceHir, TyId};
use stc_span::Span;

use crate::CodegenError;
use crate::abi::trap;
use crate::codes;
use crate::expr::{BodyInfo, Res};
use crate::ir::{Function, IrType, Value};
use crate::place::{PlaceValue, ResolvedPointer};
use crate::types::{PLACE_DESCRIPTOR_LLVM, TypeCategory, place_kind};
use crate::unit::Lowerer;

impl Lowerer<'_> {
    /// Lowers `REF(target)` into a place descriptor value.
    pub(crate) fn lower_ref(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        target: ExprId,
        span: Span,
    ) -> Res<Value> {
        let target_expr = self.hir.expr(target).clone();
        if self.types.facts(target_expr.ty)?.is_object() {
            return self.lower_object_ref(f, info, target, span);
        }
        if let stc_hir::ExprKindHir::Place(PlaceHir::Direct(address)) = &target_expr.kind {
            let kind = self.types.scalar_kind(target_expr.ty, span)?;
            let image = crate::layout::image_span(address, kind)?;
            if kind == crate::layout::ScalarKind::Bool && image.start_bit % 8 != 0 {
                return Err(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    "a reference cannot target an individual `AT` bit".into(),
                    span,
                ));
            }
            let Some(area) = address.area else {
                return Err(CodegenError::internal("direct address has no area"));
            };
            return self.build_descriptor(
                f,
                place_kind::DIRECT,
                area_service(area),
                i64::try_from(image.start_bit / 8).unwrap_or(i64::MAX),
                0,
                0,
                Value::null_ptr(),
            );
        }
        if let Some(pointer) = self.pointer_place_address(f, info, target)? {
            return self.build_descriptor(f, place_kind::STACK, 0, 0, 0, 0, pointer);
        }
        let Some(path) = self.collect_place(target) else {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "`REF` requires a place".into(),
                span,
            ));
        };
        let Some(resolved) = self.resolve_place_pointer(f, info, &path)? else {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "the reference target has no stable storage".into(),
                span,
            ));
        };
        if let Some(address) = &resolved.direct {
            let Some(area) = address.area else {
                return Err(CodegenError::internal("direct address has no area"));
            };
            return self.build_descriptor(
                f,
                place_kind::DIRECT,
                area_service(area),
                direct_byte_offset(address)?,
                0,
                0,
                Value::null_ptr(),
            );
        }
        let kind = match &path.base {
            crate::place::PlaceBase::Local(_)
            | crate::place::PlaceBase::Param(_)
            | crate::place::PlaceBase::Result(_)
            | crate::place::PlaceBase::ObjectDeref { .. } => place_kind::STACK,
            _ => place_kind::ROOT,
        };
        let (lo, hi) = self.descriptor_identity(&path);
        self.build_descriptor(f, kind, 0, 0, lo, hi, resolved.pointer)
    }

    /// Returns the split 128-bit identity of a persistent place, when known.
    fn descriptor_identity(&self, path: &crate::place::PlacePath) -> (i64, i64) {
        let Some(mut segments) = self.static_segments(path) else {
            return (0, 0);
        };
        let class_handle = self.types.class_def(path.ty).ok().flatten().is_some();
        if class_handle {
            // A class handle's root is its declaration path; an array of
            // handles owns one root for the whole array.
            while matches!(
                segments.last(),
                Some(crate::types::RootSegment::Element { .. })
            ) {
                segments.pop();
            }
        }
        let target = match &path.base {
            crate::place::PlaceBase::Global(def) => {
                Some(Self::global_leaf_target(*def, &segments, class_handle))
            }
            _ => None,
        };
        let Some(id) = target.and_then(|target| self.got.root_got_id(&target)) else {
            return (0, 0);
        };
        let bytes = id.as_bytes();
        let mut lo = [0_u8; 8];
        let mut hi = [0_u8; 8];
        lo.copy_from_slice(&bytes[..8]);
        hi.copy_from_slice(&bytes[8..]);
        (i64::from_le_bytes(lo), i64::from_le_bytes(hi))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_descriptor(
        &mut self,
        f: &mut Function,
        kind: i32,
        flags: i32,
        offset: i64,
        id_lo: i64,
        id_hi: i64,
        data: Value,
    ) -> Res<Value> {
        let ty = IrType::aggregate(PLACE_DESCRIPTOR_LLVM);
        let mut value = Value::zero(ty.clone());
        value = f.insert_value_at(&value, &Value::int(32, i128::from(kind)), &[0], ty.clone());
        value = f.insert_value_at(&value, &Value::int(32, i128::from(flags)), &[1], ty.clone());
        value = f.insert_value_at(
            &value,
            &Value::int(64, i128::from(offset)),
            &[2],
            ty.clone(),
        );
        value = f.insert_value_at(&value, &Value::int(64, i128::from(id_lo)), &[3], ty.clone());
        value = f.insert_value_at(&value, &Value::int(64, i128::from(id_hi)), &[4], ty.clone());
        value = f.insert_value_at(&value, &data, &[5], ty.clone());
        value = f.insert_value_at(&value, &Value::null_ptr(), &[6], ty);
        Ok(value)
    }

    /// Builds an `Object` place descriptor.
    ///
    /// The data field is the object record handle and the descriptor field is
    /// the object's TYPEINFO pointer (D-150; Runtime and Execution §2.6).
    pub(crate) fn build_object_descriptor(
        &mut self,
        f: &mut Function,
        handle: &Value,
        typeinfo: &Value,
    ) -> Res<Value> {
        let ty = IrType::aggregate(PLACE_DESCRIPTOR_LLVM);
        let value = self.build_descriptor(f, place_kind::OBJECT, 0, 0, 0, 0, handle.clone())?;
        Ok(f.insert_value_at(&value, typeinfo, &[6], ty))
    }

    /// Loads the kind, data, and descriptor fields of a place descriptor
    /// without validating it.
    ///
    /// Callers that must not trap (assignment attempts and interface
    /// conversions) decide what a null or malformed descriptor means.
    pub(crate) fn descriptor_parts(
        &mut self,
        f: &mut Function,
        value: &Value,
    ) -> Res<(Value, Value, Value)> {
        let desc_ty = IrType::aggregate(PLACE_DESCRIPTOR_LLVM);
        let temp = f.alloca(desc_ty.clone());
        f.store(value, &temp, None);
        let pointee = desc_ty.render();
        let kind_ptr = f.gep(&pointee, &temp, &["i64 0".into(), "i32 0".into()], true);
        let kind = f.load(IrType::Int(32), &kind_ptr, None);
        let data_ptr = f.gep(&pointee, &temp, &["i64 0".into(), "i32 5".into()], true);
        let data = f.load(IrType::Ptr, &data_ptr, None);
        let descriptor_ptr = f.gep(&pointee, &temp, &["i64 0".into(), "i32 6".into()], true);
        let descriptor = f.load(IrType::Ptr, &descriptor_ptr, None);
        Ok((kind, data, descriptor))
    }

    /// Lowers `REF(object place)` to an `Object` place descriptor (D-150).
    ///
    /// A class value with a null handle yields the null descriptor, so a
    /// reference to a null class value still compares equal to `NULL`.
    fn lower_object_ref(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        target: ExprId,
        span: Span,
    ) -> Res<Value> {
        let ty = self.hir.expr(target).ty;
        let facts = self.types.facts(ty)?.clone();
        let Some(path) = self.collect_place(target) else {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "`REF` requires an object place".into(),
                span,
            ));
        };
        let Some(value) = self.resolve_place_value(f, info, &path)? else {
            return Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "`REF` requires an object place".into(),
                span,
            ));
        };
        let handle = match value {
            PlaceValue::ClassValue(handle) => handle,
            PlaceValue::ClassSlot(slot) => f.load(IrType::Ptr, &slot, None),
            PlaceValue::FbRecord(record) => record,
            PlaceValue::Leaf(_) => {
                return Err(CodegenError::unsupported(
                    codes::CODEGEN_UNSUPPORTED,
                    "`REF` requires an object place".into(),
                    span,
                ));
            }
        };
        let desc_ty = IrType::aggregate(PLACE_DESCRIPTOR_LLVM);
        if !facts.is_class() {
            let typeinfo = f.load(IrType::Ptr, &handle, Some(8));
            return self.build_object_descriptor(f, &handle, &typeinfo);
        }
        let result = f.alloca(desc_ty.clone());
        f.store(&Value::zero(desc_ty.clone()), &result, None);
        let build = f.next_block_label();
        let done = f.next_block_label();
        let is_null = f.icmp("eq", &handle, &Value::null_ptr());
        f.cond_branch(&is_null, &done, &build);
        f.start_block(&build);
        let typeinfo = f.load(IrType::Ptr, &handle, Some(8));
        let descriptor = self.build_object_descriptor(f, &handle, &typeinfo)?;
        f.store(&descriptor, &result, None);
        f.branch(&done);
        f.start_block(&done);
        Ok(f.load(desc_ty, &result, None))
    }

    /// Converts a `REF_TO` class/FB value to an interface (D-150, Q-152).
    ///
    /// The reference's `Object` descriptor supplies the record and TYPEINFO;
    /// the interface map is searched by interface `GotId`. A null reference, a
    /// null record, or an object that does not satisfy the interface yields
    /// the all-zero interface (Q-171); the conversion never traps.
    pub(crate) fn lower_reference_to_interface(
        &mut self,
        f: &mut Function,
        value: Value,
        interface: DefId,
        pair_ty: &IrType,
    ) -> Res<Value> {
        let (kind, record, typeinfo) = self.descriptor_parts(f, &value)?;
        let result = f.alloca(pair_ty.clone());
        f.store(&Value::zero(pair_ty.clone()), &result, None);

        let check_kind = f.next_block_label();
        let check_record = f.next_block_label();
        let check_typeinfo = f.next_block_label();
        let search = f.next_block_label();
        let found = f.next_block_label();
        let done = f.next_block_label();

        let is_null = f.icmp("eq", &kind, &Value::int(32, i128::from(place_kind::NULL)));
        f.cond_branch(&is_null, &done, &check_kind);
        f.start_block(&check_kind);
        let is_object = f.icmp("eq", &kind, &Value::int(32, i128::from(place_kind::OBJECT)));
        f.cond_branch(&is_object, &check_record, &done);
        f.start_block(&check_record);
        let record_null = f.icmp("eq", &record, &Value::null_ptr());
        f.cond_branch(&record_null, &done, &check_typeinfo);
        f.start_block(&check_typeinfo);
        let typeinfo_null = f.icmp("eq", &typeinfo, &Value::null_ptr());
        f.cond_branch(&typeinfo_null, &done, &search);

        f.start_block(&search);
        let (vtable, matched) = self.interface_vtable_lookup(f, &typeinfo, interface)?;
        f.cond_branch(&matched, &found, &done);

        f.start_block(&found);
        let pair = f.insert_value_at(
            &Value::zero(pair_ty.clone()),
            &record,
            &[0],
            pair_ty.clone(),
        );
        let pair = f.insert_value_at(&pair, &vtable, &[1], pair_ty.clone());
        f.store(&pair, &result, None);
        f.branch(&done);

        f.start_block(&done);
        Ok(f.load(pair_ty.clone(), &result, None))
    }

    /// Lowers an explicit `POINTER TO T` ↔ `REF_TO T` conversion.
    pub(crate) fn lower_pointer_conversion(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        value: Value,
        from: TyId,
        to: TyId,
        span: Span,
    ) -> Res<Value> {
        let from_facts = self.types.facts(from)?.clone();
        let to_facts = self.types.facts(to)?.clone();
        match (&from_facts.category, &to_facts.category) {
            (
                crate::types::TypeCategory::Pointer { .. },
                crate::types::TypeCategory::Reference { .. },
            ) => {
                let is_null = f.icmp("eq", &value, &Value::null_ptr());
                let descriptor =
                    self.build_descriptor(f, place_kind::STACK, 0, 0, 0, 0, value.clone())?;
                let null = Value::zero(descriptor.ty.clone());
                Ok(f.select(&is_null, &null, &descriptor))
            }
            (
                crate::types::TypeCategory::Reference { .. },
                crate::types::TypeCategory::Pointer { .. },
            ) => self.reference_value_pointer(f, info, value, span),
            (crate::types::TypeCategory::Null, crate::types::TypeCategory::Pointer { .. }) => {
                Ok(Value::zero(IrType::Ptr))
            }
            (
                crate::types::TypeCategory::Pointer { .. },
                crate::types::TypeCategory::Pointer { .. },
            ) => Ok(value),
            _ => Err(CodegenError::unsupported(
                codes::CODEGEN_UNSUPPORTED,
                "only `POINTER TO T` ↔ `REF_TO T` conversions are supported".into(),
                span,
            )),
        }
    }

    /// Extracts the raw storage pointer from a reference descriptor.
    ///
    /// Null, object, `%I` input, and invalid descriptors yield `NULL`; the
    /// conversion never traps (D-145).
    fn reference_value_pointer(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        value: Value,
        _span: Span,
    ) -> Res<Value> {
        let desc_ty = IrType::aggregate(PLACE_DESCRIPTOR_LLVM);
        let pointee = desc_ty.render();
        let temp = f.alloca(desc_ty.clone());
        f.store(&value, &temp, None);
        let kind_ptr = f.gep(&pointee, &temp, &["i64 0".into(), "i32 0".into()], true);
        let kind = f.load(IrType::Int(32), &kind_ptr, None);
        let flags_ptr = f.gep(&pointee, &temp, &["i64 0".into(), "i32 1".into()], true);
        let flags = f.load(IrType::Int(32), &flags_ptr, None);
        let offset_ptr = f.gep(&pointee, &temp, &["i64 0".into(), "i32 2".into()], true);
        let offset = f.load(IrType::Int(64), &offset_ptr, None);
        let data_ptr = f.gep(&pointee, &temp, &["i64 0".into(), "i32 5".into()], true);
        let data = f.load(IrType::Ptr, &data_ptr, None);
        let is_direct = f.icmp("eq", &kind, &Value::int(32, i128::from(place_kind::DIRECT)));
        let is_null = f.icmp("eq", &kind, &Value::int(32, i128::from(place_kind::NULL)));
        let is_object = f.icmp("eq", &kind, &Value::int(32, i128::from(place_kind::OBJECT)));
        let is_input = f.icmp("eq", &flags, &Value::int(32, 1));
        let direct_input = f.binop("and", &is_direct, &is_input);
        let not_direct = f.binop("xor", &is_direct, &Value::bool(true));
        let data_null = f.icmp("eq", &data, &Value::null_ptr());
        let bad_data = f.binop("and", &not_direct, &data_null);
        let null_or_object = f.binop("or", &is_null, &is_object);
        let bad_direct = f.binop("or", &direct_input, &bad_data);
        let invalid = f.binop("or", &null_or_object, &bad_direct);
        let slot = self.import_slot(stc_got::catalog::ImportService::IoImage)?;
        let service = Self::slot_storage(f, info, slot);
        let image = f
            .call_ptr(IrType::Ptr, &service, &[flags])
            .ok_or_else(|| CodegenError::internal("io_image returned void"))?;
        let base = f.select(&is_direct, &image, &data);
        let pointer = f.gep("i8", &base, &[format!("i64 {}", offset.repr)], false);
        Ok(f.select(&invalid, &Value::null_ptr(), &pointer))
    }

    /// Resolves a reference value to the storage pointer it denotes.
    pub(crate) fn deref_pointer(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        reference: ExprId,
        span: Span,
    ) -> Res<Value> {
        let value = self.lower_expr(f, info, reference)?;
        if matches!(
            self.hir.types.get(self.hir.expr(reference).ty),
            stc_hir::Ty::Ptr(_)
        ) {
            self.check_pointer(f, &value, span)?;
            return Ok(value);
        }
        let ty = IrType::aggregate(PLACE_DESCRIPTOR_LLVM);
        let temp = f.alloca(ty.clone());
        f.store(&value, &temp, None);
        let pointee = ty.render();
        let kind_ptr = f.gep(&pointee, &temp, &["i64 0".into(), "i32 0".into()], true);
        let kind = f.load(IrType::Int(32), &kind_ptr, None);
        let flags_ptr = f.gep(&pointee, &temp, &["i64 0".into(), "i32 1".into()], true);
        let flags = f.load(IrType::Int(32), &flags_ptr, None);
        let offset_ptr = f.gep(&pointee, &temp, &["i64 0".into(), "i32 2".into()], true);
        let offset = f.load(IrType::Int(64), &offset_ptr, None);
        let data_ptr = f.gep(&pointee, &temp, &["i64 0".into(), "i32 5".into()], true);
        let data = f.load(IrType::Ptr, &data_ptr, None);
        let is_direct = f.icmp("eq", &kind, &Value::int(32, i128::from(place_kind::DIRECT)));
        let is_null = f.icmp("eq", &kind, &Value::int(32, i128::from(place_kind::NULL)));
        let data_null = f.icmp("eq", &data, &Value::null_ptr());
        let not_direct = f.binop("xor", &is_direct, &Value::bool(true));
        let bad_data = f.binop("and", &not_direct, &data_null);
        let invalid = f.binop("or", &is_null, &bad_data);
        self.trap_if(f, &invalid, trap::NULL_REF, span)?;
        let slot = self.import_slot(stc_got::catalog::ImportService::IoImage)?;
        let service = Self::slot_storage(f, info, slot);
        let image = f
            .call_ptr(IrType::Ptr, &service, &[flags])
            .ok_or_else(|| CodegenError::internal("io_image returned void"))?;
        let base = f.select(&is_direct, &image, &data);
        Ok(f.gep("i8", &base, &[format!("i64 {}", offset.repr)], false))
    }
}

fn area_service(area: stc_hir::DirectArea) -> i32 {
    match area {
        stc_hir::DirectArea::Input => 1,
        stc_hir::DirectArea::Output => 2,
        stc_hir::DirectArea::Memory => 3,
    }
}

pub(crate) fn direct_byte_offset(address: &stc_hir::DirectAddress) -> Res<i64> {
    // The image span depends on the target type, which the caller validated
    // when the direct address was resolved. Bits are not addressable here.
    let indices = &address.indices;
    let (byte, bit) = match (address.size, indices.as_slice()) {
        (Some(stc_hir::DirectSize::Bit), &[byte, bit]) => (u64::from(byte), u64::from(bit)),
        (Some(stc_hir::DirectSize::Byte), &[byte]) => (u64::from(byte), 0),
        (Some(stc_hir::DirectSize::Word), &[word]) => (u64::from(word) * 2, 0),
        (Some(stc_hir::DirectSize::DWord), &[word]) => (u64::from(word) * 4, 0),
        (Some(stc_hir::DirectSize::LWord), &[word]) => (u64::from(word) * 8, 0),
        (None, &[byte, bit]) => (u64::from(byte), u64::from(bit)),
        (None, &[byte]) => (u64::from(byte), 0),
        _ => {
            return Err(CodegenError::internal(
                "malformed direct address in reference",
            ));
        }
    };
    if bit != 0 {
        return Err(CodegenError::unsupported(
            codes::CODEGEN_UNSUPPORTED,
            "a reference cannot target an `AT` bit".into(),
            address.span,
        ));
    }
    i64::try_from(byte)
        .map_err(|_| CodegenError::internal("direct address byte offset exceeds i64"))
}

impl Lowerer<'_> {
    /// Reads through a dereference place.
    pub(crate) fn read_deref(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        reference: ExprId,
        ty: TyId,
        span: Span,
    ) -> Res<Value> {
        let facts = self.types.facts(ty)?.clone();
        if facts.is_object() {
            let record = self.deref_object_record(f, info, reference, span)?;
            if facts.is_class() {
                return Ok(record);
            }
            return self.gather_fb(f, info, ty, &record);
        }
        let pointer = self.deref_pointer(f, info, reference, span)?;
        if let Some(kind) = facts.scalar() {
            return Self::read_scalar(f, &pointer, kind);
        }
        Ok(f.load(facts.llvm_storage, &pointer, None))
    }

    /// Writes through a dereference place.
    pub(crate) fn write_deref(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        reference: ExprId,
        ty: TyId,
        value: Value,
        span: Span,
    ) -> Res<()> {
        let facts = self.types.facts(ty)?.clone();
        if facts.is_object() {
            let record = self.deref_object_record(f, info, reference, span)?;
            if facts.is_class() {
                // Assignment through an `Object` reference copies state into
                // the referenced object; it never rebinds the original
                // variable's handle (D-150).
                self.check_pointer(f, &value, span)?;
                return self.copy_object_state(f, ty, &record, &value);
            }
            return self.split_fb(f, info, ty, &record, &value);
        }
        let pointer = self.deref_pointer(f, info, reference, span)?;
        if let Some(kind) = facts.scalar() {
            self.check_subrange(f, ty, &value, span)?;
            Self::write_scalar(f, &pointer, kind, &value);
            return Ok(());
        }
        f.store(&value, &pointer, None);
        Ok(())
    }

    /// Loads the object record named by a dereferenced `Object` descriptor.
    ///
    /// The record pointer is the descriptor's data field; a null reference
    /// traps `STC-TRAP_NULL_REF` (`^` dereferences, unlike `?=`).
    fn deref_object_record(
        &mut self,
        f: &mut Function,
        info: &BodyInfo,
        reference: ExprId,
        span: Span,
    ) -> Res<Value> {
        let value = self.lower_expr(f, info, reference)?;
        let (_, record, _) = self.descriptor_parts(f, &value)?;
        self.check_pointer(f, &record, span)?;
        Ok(record)
    }

    /// Copies one object's leaf state into another record of the same
    /// static type, leaving both handles untouched.
    fn copy_object_state(
        &mut self,
        f: &mut Function,
        ty: TyId,
        destination: &Value,
        source: &Value,
    ) -> Res<()> {
        let def = match self.types.category(ty)? {
            TypeCategory::Object { def } => *def,
            _ => return Err(CodegenError::internal("object state on a non-object")),
        };
        let layout = self
            .objects
            .get(&def)
            .cloned()
            .ok_or_else(|| CodegenError::internal("object type has no leaf layout"))?;
        let ordinals = self.object_ordinals_for(def)?;
        let destination_table = Self::object_table_pointer(f, destination);
        let source_table = Self::object_table_pointer(f, source);
        for leaf in &layout.leaves {
            let source_pointer = Self::table_entry(f, &source_table, &ordinals, &leaf.segments);
            let value = self.read_leaf(
                f,
                &ResolvedPointer {
                    pointer: source_pointer,
                    direct: None,
                },
                leaf.ty,
            )?;
            let destination_pointer =
                Self::table_entry(f, &destination_table, &ordinals, &leaf.segments);
            self.write_leaf(
                f,
                &ResolvedPointer {
                    pointer: destination_pointer,
                    direct: None,
                },
                leaf.ty,
                value,
            )?;
        }
        Ok(())
    }
}

impl Lowerer<'_> {
    /// Compares reference descriptors for `=` / `<>`.
    pub(crate) fn lower_reference_comparison(
        &mut self,
        f: &mut Function,
        lhs_ty: TyId,
        lhs: &Value,
        rhs_ty: TyId,
        rhs: &Value,
        equal: bool,
    ) -> Res<Value> {
        let lhs_facts = self.types.facts(lhs_ty)?.clone();
        let rhs_facts = self.types.facts(rhs_ty)?.clone();
        let desc_ty = IrType::aggregate(PLACE_DESCRIPTOR_LLVM);
        let pointee = desc_ty.render();
        let lhs_null = matches!(lhs_facts.category, TypeCategory::Null);
        let rhs_null = matches!(rhs_facts.category, TypeCategory::Null);
        if lhs_null || rhs_null {
            let (descriptor, null_side) = if lhs_null {
                (rhs, rhs_null)
            } else {
                (lhs, lhs_null)
            };
            let _ = null_side;
            let temp = f.alloca(desc_ty.clone());
            f.store(descriptor, &temp, None);
            let kind_ptr = f.gep(&pointee, &temp, &["i64 0".into(), "i32 0".into()], true);
            let kind = f.load(IrType::Int(32), &kind_ptr, None);
            let is_null = f.icmp("eq", &kind, &Value::int(32, 0));
            return Ok(if equal {
                is_null
            } else {
                f.binop("xor", &is_null, &Value::bool(true))
            });
        }
        let lhs_temp = f.alloca(desc_ty.clone());
        f.store(lhs, &lhs_temp, None);
        let rhs_temp = f.alloca(desc_ty.clone());
        f.store(rhs, &rhs_temp, None);
        let mut same = Value::bool(true);
        for index in 0..7 {
            let field_ty = if index == 5 || index == 6 {
                IrType::Ptr
            } else if index == 0 || index == 1 {
                IrType::Int(32)
            } else {
                IrType::Int(64)
            };
            let lhs_ptr = f.gep(
                &pointee,
                &lhs_temp,
                &["i64 0".into(), format!("i32 {index}")],
                true,
            );
            let rhs_ptr = f.gep(
                &pointee,
                &rhs_temp,
                &["i64 0".into(), format!("i32 {index}")],
                true,
            );
            let lhs_field = f.load(field_ty.clone(), &lhs_ptr, None);
            let rhs_field = f.load(field_ty, &rhs_ptr, None);
            let matches = f.icmp("eq", &lhs_field, &rhs_field);
            same = f.binop("and", &same, &matches);
        }
        Ok(if equal {
            same
        } else {
            f.binop("xor", &same, &Value::bool(true))
        })
    }
}
