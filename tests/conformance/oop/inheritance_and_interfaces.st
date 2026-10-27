INTERFACE IBase
METHOD Base : DINT
END_METHOD
END_INTERFACE
CLASS BaseImpl IMPLEMENTS IBase
VAR PUBLIC
  id : DINT;
END_VAR
METHOD PUBLIC Base : DINT
  Base := id;
END_METHOD
END_CLASS
CLASS DerivedImpl EXTENDS BaseImpl
VAR PUBLIC
  extra : DINT;
END_VAR
METHOD PUBLIC OVERRIDE Base : DINT
  Base := SUPER.Base() + extra;
END_METHOD
END_CLASS
PROGRAM Main
VAR
  d : DerivedImpl;
  b : BaseImpl;
  i : IBase;
  x : DINT;
  zero : DINT;
END_VAR
  d.id := 3;
  d.extra := 4;
  b.id := 2;
  i := d;
  x := i.Base();
  IF x <> 7 THEN
    zero := 0;
    x := 1 / zero;
  END_IF;
  b := d;
  x := b.Base();
  IF x <> 7 THEN
    zero := 0;
    x := 1 / zero;
  END_IF;
  b ?= d;
  IF b = NULL THEN
    zero := 0;
    x := 1 / zero;
  END_IF;
END_PROGRAM
