// SYNTAX TEST "source.st" "declaration names"

FUNCTION AddStep : DINT
// <------- keyword.declaration.st
//       ^^^^^^^ entity.name.function.st
//                 ^^^^ support.type.primitives.st

FUNCTION Compute : ResultType
// <------- keyword.declaration.st
//       ^^^^^^^ entity.name.function.st
//                 ^^^^^^^^^^ entity.name.type.st

PROGRAM Main
// <------- keyword.declaration.st
//      ^^^^ entity.name.function.st

FUNCTION_BLOCK FINAL Accumulator
// <------------- keyword.declaration.st
//             ^^^^^ storage.modifier.st
//                   ^^^^^^^^^^^ entity.name.type.st

CLASS ABSTRACT Base
// <---- keyword.declaration.st
//    ^^^^^^^^ storage.modifier.st
//             ^^^^ entity.name.type.st

INTERFACE IFoo
// <-------- keyword.declaration.st
//         ^^^^ entity.name.type.st

METHOD PUBLIC FINAL OVERRIDE Reset : BOOL
// <----- keyword.declaration.st
//     ^^^^^^ storage.modifier.st
//            ^^^^^ storage.modifier.st
//                  ^^^^^^^^ storage.modifier.st
//                           ^^^^^ entity.name.function.st
//                                   ^^^^ support.type.primitives.st

ACTION Reset
// <----- keyword.declaration.st
//     ^^^^^ entity.name.function.st

NAMESPACE Motor.Types
// <-------- keyword.declaration.st
//        ^^^^^^^^^^^ entity.name.namespace.st

NAMESPACE INTERNAL Internal.Name
// <-------- keyword.declaration.st
//        ^^^^^^^^ storage.modifier.st
//                 ^^^^^^^^^^^^^ entity.name.namespace.st

USING Motor.Types, Shared
// <--- keyword.declaration.st
//    ^^^^^^^^^^^^^^^^^^^ entity.name.namespace.st

CONFIGURATION Sample
// <------------ keyword.declaration.st
//            ^^^^^^ entity.name.namespace.st

RESOURCE Cpu ON Cpu
// <------ keyword.declaration.st
//       ^^^ entity.name.namespace.st
//           ^^ keyword.declaration.st
//              ^^^ entity.name.namespace.st

  TASK MainTask(INTERVAL := T#10ms, PRIORITY := 1);
//^^^^ keyword.declaration.st
//     ^^^^^^^^ variable.name.st
//     ^^^^^^^^ - entity.name.function.st
//              ^^^^^^^^ keyword.declaration.st
//                                  ^^^^^^^^ keyword.declaration.st

  PROGRAM sample WITH MainTask : Main;
//^^^^^^^ keyword.declaration.st
//        ^^^^^^ variable.name.st
//        ^^^^^^ - entity.name.function.st
//               ^^^^ keyword.declaration.st
//                    ^^^^^^^^ variable.name.st
//                               ^^^^ entity.name.type.st
//                               ^^^^ - variable.name.st

TYPE
// <---- keyword.declaration.st
  Point : STRUCT
//^^^^^ entity.name.type.st
//        ^^^^^^ keyword.declaration.st
    x : DINT;
//  ^ variable.other.member.st
//  ^ - variable.parameter.st
  END_STRUCT
//^^^^^^^^^^ keyword.declaration.st
  Color : (Red, Green);
//^^^^^ entity.name.type.st
//         ^^^ variable.other.enummember.st
//              ^^^^^ variable.other.enummember.st
END_TYPE
// <------- keyword.declaration.st
// <------- - entity.name.type.st

END_FUNCTION
// <------------ keyword.declaration.st
// <------------ - entity.name.function.st
