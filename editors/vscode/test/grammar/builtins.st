// SYNTAX TEST "source.st" "built-in functions and standard function blocks"

PROGRAM Main
VAR
  result : DINT;
  timer : TON;
//        ^^^ support.class.st
//        ^^^ - storage.type.st
//        ^^^ - entity.name.type.st
END_VAR
  result := ABS(result);
//          ^^^ support.function.st
//          ^^^ - keyword.operator.st
//          ^^^ - entity.name.function.st
  result := add(result, 1);
//          ^^^ support.function.st
  result := AddStep(result);
//          ^^^^^^^ variable.name.st
//          ^^^^^^^ - support.function.st
//          ^^^^^^^ - entity.name.function.st
  timer(IN := TRUE, PT := T#1s);
//^^^^^ variable.name.st
//^^^^^ - support.class.st
//      ^^ variable.name.st
//      ^^ - variable.parameter.st
//            ^^^^ constant.language.st
//                        ^^^^ constant.numeric.duration.st
END_PROGRAM

FUNCTION Test : DINT
  STC_ASSERT(TRUE);
//^^^^^^^^^^ support.function.st
//^^^^^^^^^^ - keyword.operator.st
//           ^^^^ constant.language.st
  Test := 1;
//^^^^ variable.name.st
//^^^^ - support.function.st
//^^^^ - entity.name.function.st
END_FUNCTION
