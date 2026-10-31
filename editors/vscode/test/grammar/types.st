// SYNTAX TEST "source.st" "type references"

FUNCTION_BLOCK Derived EXTENDS Base IMPLEMENTS IFoo, IBar
// <------------- keyword.declaration.st
//             ^^^^^^^ entity.name.type.st
//                     ^^^^^^^ keyword.declaration.st
//                             ^^^^ entity.other.inherited-class.st
//                                  ^^^^^^^^^^ keyword.declaration.st
//                                             ^^^^ entity.other.inherited-class.st
//                                                   ^^^^ entity.other.inherited-class.st

FUNCTION Compute : DINT
VAR_INPUT
  int_value : DINT;
//            ^^^^ support.type.primitives.st
//            ^^^^ - storage.type.st
  custom : Point;
//         ^^^^^ entity.name.type.st
  list : ARRAY[1..4] OF DINT;
//       ^^^^^ keyword.declaration.st
//                   ^^ keyword.operator.type.st
//                   ^^ - keyword.control.conditional.st
//                   ^^ - keyword.control.loop.st
//                      ^^^^ support.type.primitives.st
  items : ARRAY[1..4] OF Item;
//                       ^^^^ entity.name.type.st
  ref : REF_TO DINT;
//      ^^^^^^ keyword.declaration.st
//             ^^^^ support.type.primitives.st
  ptr : POINTER TO DINT;
//      ^^^^^^^ keyword.declaration.st
//              ^^ keyword.operator.type.st
//              ^^ - keyword.control.loop.st
//                 ^^^^ support.type.primitives.st
  dotted : ns.Sub.Type;
//         ^^^^^^^^^^^ entity.name.type.st
END_VAR
END_FUNCTION

PROGRAM Main
VAR
  x : DINT;
END_VAR
  CASE x OF
//^^^^ keyword.control.conditional.st
//     ^ variable.name.st
//       ^^ keyword.control.conditional.st
//       ^^ - keyword.operator.type.st
    1 : x := 2;
//  ^ constant.numeric.integer.st
//  ^ - entity.name.type.st
  END_CASE;
//^^^^^^^^ keyword.control.conditional.st
  FOR i := 1 TO 10 DO
//^^^ keyword.control.loop.st
//    ^ variable.name.st
//           ^^ keyword.control.loop.st
//           ^^ - keyword.operator.type.st
//                 ^^ keyword.control.loop.st
  END_FOR;
//^^^^^^^ keyword.control.loop.st
END_PROGRAM
