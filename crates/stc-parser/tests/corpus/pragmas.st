{stc:test {"cycles": 25}}
FUNCTION TestAdd : BOOL
VAR
  r : DINT;
END_VAR
  r := Add(2, 3);
  TestAdd := r = 5;
END_FUNCTION

VAR_GLOBAL
  MyBool AT %IX5 : BOOL {stc:io {"device": "drive1", "channel": 5, "direction": "input"}};
  Hidden : INT {stc:deprecated {"message": "use MyBool"}};
END_VAR
