(* Two tests covering disjoint parts of the production source: `AddWorks`
   covers `AddValues`, and `ClassifyPositive` covers the true branch of `Classify`.
   `Average` and the false branch of `Classify` stay uncovered. *)

{stc:test}
FUNCTION AddWorks : BOOL
VAR
  result : DINT;
END_VAR
  result := AddValues(2, 3);
  STC_ASSERT(result = 5, "AddValues should sum its inputs");
  AddWorks := TRUE;
END_FUNCTION

{stc:test}
FUNCTION ClassifyPositive : BOOL
VAR
  result : DINT;
END_VAR
  result := Classify(7);
  STC_ASSERT(result = 1, "a non-negative value should classify as 1");
  ClassifyPositive := TRUE;
END_FUNCTION
