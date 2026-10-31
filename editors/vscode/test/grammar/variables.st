// SYNTAX TEST "source.st" "variables parameters and members"

FUNCTION AddStep : DINT
VAR_INPUT
  value : DINT;
//^^^^^ variable.parameter.st
//^^^^^ - variable.name.st
//        ^^^^ support.type.primitives.st
END_VAR
VAR
  incremented : DINT;
//^^^^^^^^^^^ variable.name.st
//^^^^^^^^^^^ - variable.parameter.st
END_VAR
  incremented := value + 1;
//^^^^^^^^^^^ variable.name.st
//               ^^^^^ variable.name.st
END_FUNCTION

TYPE
  Point : STRUCT
    x : DINT;
//  ^ variable.other.member.st
//  ^ - variable.parameter.st
  END_STRUCT
END_TYPE

PROGRAM Main
VAR
  accumulator : Accumulator;
//^^^^^^^^^^^ variable.name.st
//              ^^^^^^^^^^^ entity.name.type.st
  total : DINT;
//^^^^^ variable.name.st
END_VAR
  accumulator.total := 1;
//^^^^^^^^^^^ variable.name.st
//            ^^^^^ variable.other.member.st
//            ^^^^^ - variable.parameter.st
  accumulator(increment := 2);
//^^^^^^^^^^^ variable.name.st
//            ^^^^^^^^^ variable.name.st
//            ^^^^^^^^^ - variable.parameter.st
  AddStep(total);
//^^^^^^^ variable.name.st
//^^^^^^^ - entity.name.function.st
//^^^^^^^ - support.function.st
//        ^^^^^ variable.name.st
END_PROGRAM

METHOD Reset
  THIS.value := SUPER.value;
//^^^^ variable.language.st
//^^^^ - variable.name.st
//     ^^^^^ variable.other.member.st
//              ^^^^^ variable.language.st
//                    ^^^^^ variable.other.member.st
END_METHOD
