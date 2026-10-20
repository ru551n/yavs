-- difftest: top=aggregate_elem_lengths std=2008 exit=1 ignore=fatalmsg,path
-- ignore=fatalmsg,path: the references word the length error and name its process differently.
-- The elements of an aggregate whose element subtype is an unconstrained array must all have
-- the same length (LRM 9.3.3.3); a constant aggregate of string literals of other lengths is
-- not folded into one value that hides it, the run-time check fails.
entity aggregate_elem_lengths is end;
architecture t of aggregate_elem_lengths is
  type strings is array (natural range <>) of string;
  type rec is record
    s : string;
    n : integer;
  end record;
  type recs is array (natural range <>) of rec;
  function mk (a, b : string) return strings is
  begin
    return (a, b);
  end function;
  constant same : strings(0 to 1) := ("one", "two");
begin
  process
  begin
    report same(0) & same(1);
    report mk("ab", "cd")(1);
    report mk("one", "three")(1);
    wait;
  end process;
end;
