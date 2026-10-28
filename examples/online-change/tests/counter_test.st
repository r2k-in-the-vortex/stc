{stc:test}
FUNCTION CounterIncrements : BOOL
VAR
  counter : Counter;
  before : DINT;
END_VAR
  counter();
  before := counter.Count;
  counter();
  STC_ASSERT(counter.Count = before + INCREMENT, "Count should advance by INCREMENT each scan");
  CounterIncrements := TRUE;
END_FUNCTION

{stc:test}
FUNCTION RetainedTotalAccumulates : BOOL
VAR
  counter : Counter;
  before : DINT;
END_VAR
  counter();
  before := counter.Retained;
  counter();
  STC_ASSERT(counter.Retained = before + INCREMENT, "Retained should accumulate across scans");
  RetainedTotalAccumulates := TRUE;
END_FUNCTION
