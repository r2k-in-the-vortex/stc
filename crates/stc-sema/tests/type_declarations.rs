//! Type-declaration coverage: named values, inline structs, enums, subranges,
//! interfaces, abstract/final rules, vtables, and direct addresses.

mod common;

use common::{assert_clean, assert_error, assert_errors};
use stc_sema::codes;

#[test]
fn named_value_types_resolve_and_combine() {
    assert_clean(
        "TYPE Flags : DWORD (Read := 16#01, Write := 16#02); END_TYPE
PROGRAM P
VAR
  d : DWORD;
  b : BOOL;
END_VAR
  d := Read;
  b := d = Read;
END_PROGRAM
",
    );
    assert_error(
        "TYPE Flags : DWORD (Read := 16#01); END_TYPE
PROGRAM P
VAR d : DWORD; END_VAR
  d := Nope;
END_PROGRAM
",
        codes::NAME_UNKNOWN,
    );
    assert_error(
        "TYPE Flags : REAL (Read := 1.0); END_TYPE
",
        codes::TYPE_INVALID,
    );
}

#[test]
fn inline_struct_types_resolve() {
    assert_clean(
        "PROGRAM P
VAR
  s : STRUCT
    x : DINT;
    y : DINT;
  END_STRUCT;
  x : DINT;
END_VAR
  s.x := 1;
  s.y := 2;
  x := s.x + s.y;
END_PROGRAM
",
    );
}

#[test]
fn enum_and_subrange_declarations_are_checked() {
    assert_clean(
        "TYPE
  Color : (Red, Green := 2, Blue);
  Speed : INT (0..100);
END_TYPE
PROGRAM P
VAR
  c : Color;
  s : Speed;
END_VAR
  c := Green;
  s := 50;
END_PROGRAM
",
    );
    assert_error("TYPE E : (A, A); END_TYPE\n", codes::NAME_DUPLICATE);
    assert_error("TYPE S : INT (10..0); END_TYPE\n", codes::CONST_FAILURE);
}

#[test]
fn struct_declarations_are_checked() {
    assert_error(
        "TYPE S : STRUCT x : DINT; x : DINT; END_STRUCT; END_TYPE\n",
        codes::NAME_DUPLICATE,
    );
    assert_error(
        "TYPE S : STRUCT x : Missing; END_STRUCT; END_TYPE\n",
        codes::NAME_UNKNOWN,
    );
}

#[test]
fn interface_and_abstract_rules_are_enforced() {
    assert_clean(
        "INTERFACE IShape
METHOD Area : DINT
END_METHOD
END_INTERFACE
CLASS Rect IMPLEMENTS IShape
VAR PUBLIC
  w : DINT;
END_VAR
METHOD PUBLIC Area : DINT
  Area := w * w;
END_METHOD
END_CLASS
CLASS ABSTRACT Base IMPLEMENTS IShape
END_CLASS
CLASS Concrete EXTENDS Base
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC Area : DINT
  Area := id;
END_METHOD
END_CLASS
PROGRAM P
VAR
  r : Rect;
  x : DINT;
END_VAR
  x := r.Area();
END_PROGRAM
",
    );
    assert_error(
        "INTERFACE I\nMETHOD M : DINT\nEND_METHOD\nEND_INTERFACE\nCLASS C IMPLEMENTS I\nEND_CLASS\n",
        codes::OOP_INTERFACE,
    );
}

#[test]
fn direct_address_declarations_are_checked() {
    assert_clean(
        "PROGRAM P
VAR
  in_val AT %IX0.0 : BOOL;
  w_val AT %IW1 : WORD;
  out_val AT %QW2 : WORD;
  m_val AT %MD3 : DINT;
END_VAR
  out_val := w_val;
  m_val := m_val + 1;
END_PROGRAM
",
    );
    let cases: &[&str] = &[
        "PROGRAM P\nVAR\n  x AT %MW0 : LINT;\nEND_VAR\nEND_PROGRAM\n",
        "PROGRAM P\nVAR\n  x AT %IX0.0 : INT;\nEND_VAR\nEND_PROGRAM\n",
    ];
    for source in cases {
        assert_error(source, codes::IO_ADDRESS);
    }
}

#[test]
fn class_assignment_and_vtable_rules_are_enforced() {
    assert_errors(
        "INTERFACE IShape
METHOD Area : DINT
END_METHOD
END_INTERFACE
CLASS Rect IMPLEMENTS IShape
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC Area : DINT
  Area := id;
END_METHOD
END_CLASS
CLASS Circle IMPLEMENTS IShape
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC Area : DINT
  Area := id;
END_METHOD
END_CLASS
PROGRAM P
VAR
  r : Rect;
  c : Circle;
  i : IShape;
  x : DINT;
END_VAR
  i := r;
  i := c;
  x := i.Area();
  i := NULL;
END_PROGRAM
",
        &[],
    );
    assert_error(
        "CLASS Unrelated
VAR PUBLIC
  id : DINT;
END_VAR
END_CLASS
INTERFACE I
METHOD M : DINT
END_METHOD
END_INTERFACE
PROGRAM P
VAR
  u : Unrelated;
  i : I;
END_VAR
  i := u;
END_PROGRAM
",
        codes::TYPE_ASSIGN,
    );
}
