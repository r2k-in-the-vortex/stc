//! Constant evaluation coverage: literal kinds, operators, conversions,
//! aggregate initializers, and constant paths.

mod common;

use common::{assert_clean, assert_error};
use stc_sema::codes;

#[test]
fn constant_literals_operators_and_initializers_fold() {
    assert_clean(
        "TYPE
  Color : (Red, Green := 5, Blue);
  Small : INT(0..10);
  Alias : DINT;
  Point : STRUCT
    x : DINT;
    y : DINT;
  END_STRUCT;
END_TYPE
VAR_GLOBAL CONSTANT
  I_DEC : DINT := 10;
  I_HEX : DINT := 16#FF;
  I_BIN : UINT := 2#1010_1010;
  I_TYPED : DINT := DINT#16#10;
  I_SINT : SINT := SINT#-1;
  I_BOOL : BOOL := 1;
  I_BOOL2 : BOOL := TRUE;
  I_CHAR : CHAR := CHAR#'a';
  I_WCHAR : WCHAR := WCHAR#'b';
  I_UCHAR : UCHAR := UCHAR#'c';
  R1 : REAL := 1.5;
  R2 : REAL := 1;
  R3 : LREAL := LREAL#2.5;
  T1 : TIME := T#1s;
  T1B : TIME := T#500ms;
  T1C : TIME := T#1m30s;
  T2 : LTIME := LTIME#2s;
  D1 : DATE := D#2024-01-02;
  D2 : LDATE := LDATE#2024-01-02;
  TOD1 : TOD := TOD#01:02:03;
  TOD2 : LTOD := LTOD#01:02:03;
  DT1 : DT := DT#2024-01-02-01:02:03;
  DT2 : LDT := LDT#2024-01-02-01:02:03;
  S1 : STRING[8] := 'hello';
  S2 : WSTRING[8] := WSTRING#\"wide\";
  S3 : USTRING[8] := USTRING#'utf';
  NEG : DINT := -5;
  POS : DINT := +5;
  PAREN : DINT := (2 + 3) * 4;
  POW : DINT := 2 ** 8;
  MODC : DINT := 17 MOD 5;
  DIVC : DINT := 17 / 5;
  BITS_AND : BYTE := BYTE#16#0C AND BYTE#16#0A;
  BITS_OR : BYTE := BYTE#16#0C OR BYTE#16#0A;
  BITS_XOR : BYTE := BYTE#16#0C XOR BYTE#16#0A;
  BITS_NOT : BYTE := NOT BYTE#16#0F;
  CMP_LT : BOOL := 1 < 2;
  CMP_LE : BOOL := 1 <= 2;
  CMP_GT : BOOL := 1 > 2;
  CMP_GE : BOOL := 1 >= 2;
  CMP_EQ : BOOL := 1 = 2;
  CMP_NE : BOOL := 1 <> 2;
  BOOL_AND : BOOL := TRUE AND FALSE;
  BOOL_OR : BOOL := TRUE OR FALSE;
  BOOL_XOR : BOOL := TRUE XOR FALSE;
  BOOL_NOT : BOOL := NOT TRUE;
  REAL_ADD : REAL := 1.5 + 2.5;
  REAL_SUB : REAL := 2.5 - 1.5;
  REAL_MUL : REAL := 1.5 * 2.5;
  REAL_DIV : REAL := 1.5 / 2.5;
  SUBC : Small := 5;
  ENUMC : Color := Green;
  ALIASC : Alias := 7;
  PATHS : DINT := I_DEC + I_HEX + NEG;
  CONV1 : LINT := DINT_TO_LINT(10);
  CONV2 : REAL := DINT_TO_REAL(3);
  CONV3 : UDINT := DINT_TO_UDINT(DINT#-1);
  ARR : ARRAY[1..2] OF DINT := [1, 2];
  PT : Point := (x := 1, y := 2);
END_VAR
PROGRAM P
VAR
  x : DINT;
  r : REAL;
  s : STRING[8];
END_VAR
  x := I_DEC + PATHS + SUBC + ALIASC + CONV1;
  r := R1 + R2 + REAL_ADD;
  s := S1;
END_PROGRAM
",
    );
}

#[test]
fn non_constant_initializers_are_reported() {
    assert_error(
        "VAR_GLOBAL CONSTANT
  k : DINT := CONCAT('a', 'b');
END_VAR
",
        codes::CONST_NOT_CONSTANT,
    );
    assert_error(
        "VAR_GLOBAL
  seed : DINT;
END_VAR
VAR_GLOBAL CONSTANT
  k : DINT := seed;
END_VAR
",
        codes::CONST_NOT_CONSTANT,
    );
}

#[test]
fn constant_failures_are_reported() {
    for source in [
        "VAR_GLOBAL CONSTANT\n  k : DINT := 1 / 0;\nEND_VAR\n",
        "VAR_GLOBAL CONSTANT\n  k : DINT := DINT#2147483647 + 1;\nEND_VAR\n",
        "VAR_GLOBAL CONSTANT\n  k : BOOL := 2;\nEND_VAR\n",
        "VAR_GLOBAL CONSTANT\n  k : SINT := 200;\nEND_VAR\n",
        "VAR_GLOBAL CONSTANT\n  k : UINT := -1;\nEND_VAR\n",
    ] {
        assert_error(source, codes::CONST_FAILURE);
    }
}

#[test]
fn string_encoding_mismatches_are_reported() {
    assert_error(
        "VAR_GLOBAL CONSTANT
  k : WSTRING[4] := 'a';
END_VAR
",
        codes::TYPE_MISMATCH,
    );
}

#[test]
fn constant_adaptation_and_typed_literals_fold() {
    assert_clean(
        "TYPE
  Color : (Red, Green);
  Small : INT(0..10);
END_TYPE
VAR_GLOBAL CONSTANT
  BOOL_ZERO : BOOL := 0;
  BOOL_ONE : BOOL := 1;
  REAL_FROM_INT : REAL := 3;
  LREAL_FROM_REAL : LREAL := REAL#1.5;
  BYTE_FROM_INT : BYTE := 255;
  WORD_FROM_INT : WORD := 65535;
  S8 : SINT := SINT#127;
  I16 : INT := INT#32767;
  U8 : USINT := USINT#255;
  U16 : UINT := UINT#65535;
  U32 : UDINT := UDINT#4294967295;
  U64 : ULINT := ULINT#18446744073709551615;
  DW : DWORD := DWORD#16#FFFFFFFF;
  LW : LWORD := LWORD#16#FFFFFFFFFFFFFFFF;
  R : REAL := REAL#1.5;
  LR : LREAL := LREAL#2.5;
  T1 : TIME := TIME#1s;
  T2 : LTIME := LTIME#2s;
  D1 : DATE := DATE#2024-01-02;
  D2 : LDATE := LDATE#2024-01-02;
  TOD1 : TOD := TOD#01:02:03;
  TOD2 : LTOD := LTOD#01:02:03;
  DT1 : DT := DT#2024-01-02-01:02:03;
  DT2 : LDT := LDT#2024-01-02-01:02:03;
  CH : CHAR := CHAR#'a';
  WCH : WCHAR := WCHAR#'b';
  UCH : UCHAR := UCHAR#'c';
  ENUM_VALUE : Color := Red;
  SMALL_VALUE : Small := 5;
  NOT_BOOL : BOOL := NOT FALSE;
  NOT_BITS : WORD := NOT WORD#16#0F;
  NEG_REAL : REAL := -REAL#1.5;
  NEG_TIME : TIME := -T#1s;
  PLUS_INT : DINT := +5;
  REAL_POW : REAL := 2.0 ** 3.0;
  REAL_DIV : REAL := 1.5 / 2.5;
  BIT_AND : WORD := WORD#16#F0 AND WORD#16#0F;
  BIT_OR : WORD := WORD#16#F0 OR WORD#16#0F;
  BIT_XOR : WORD := WORD#16#F0 XOR WORD#16#0F;
  BOOL_AND : BOOL := TRUE AND FALSE;
  BOOL_OR : BOOL := TRUE OR FALSE;
  BOOL_XOR : BOOL := TRUE XOR FALSE;
  CMP_CHAR : BOOL := CHAR#'a' < CHAR#'b';
  CMP_TIME : BOOL := T#1s < T#2s;
  CMP_DATE : BOOL := D#2024-01-02 = D#2024-01-02;
  CMP_ENUM : BOOL := Red <> Green;
  CMP_BITS : BOOL := BYTE#1 <= BYTE#2;
  CONV_BOOL : BOOL := DINT_TO_BOOL(1);
  CONV_INT_BOOL : INT := BOOL_TO_INT(TRUE);
  CONV_SINT : SINT := DINT_TO_SINT(100);
  CONV_ULINT : ULINT := DINT_TO_ULINT(10);
  STRING_CAPACITY : STRING[8] := 'fits';
END_VAR
",
    );
}

#[test]
fn constant_adaptation_failures_are_reported() {
    for source in [
        "VAR_GLOBAL CONSTANT\n  k : BOOL := 2;\nEND_VAR\n",
        "VAR_GLOBAL CONSTANT\n  k : SINT := 128;\nEND_VAR\n",
        "VAR_GLOBAL CONSTANT\n  k : STRING[2] := 'long';\nEND_VAR\n",
        "VAR_GLOBAL CONSTANT\n  k : REAL := 1.0 MOD 2.0;\nEND_VAR\n",
        "VAR_GLOBAL CONSTANT\n  k : REAL := 1.0 / 0.0;\nEND_VAR\n",
    ] {
        assert_error(source, codes::CONST_FAILURE);
    }
    assert_error(
        "TYPE Color : (Red, Green); END_TYPE\nVAR_GLOBAL CONSTANT\n  k : Color := 1;\nEND_VAR\n",
        codes::TYPE_MISMATCH,
    );
}
