PROGRAM Expressions
VAR
  a, b, c : DINT;
  flag : BOOL;
  arr : ARRAY [0..9, 0..9] OF DINT;
  p : REF_TO DINT;
  obj : Object;
END_VAR
  a := A = B < C;
  b := -2 ** 2 + 2 ** 3 ** 4;
  c := (a + b) * c / 2 MOD 3;
  flag := NOT A = B AND C OR D XOR E;
  a := arr[1, 2] + arr[1][2] + p^ + THIS.b + obj.member + a.0 + a.%X0;
  b := f(1, named := 2, out => a, NOT flag => b) + REF(a) + NULL;
END_PROGRAM
