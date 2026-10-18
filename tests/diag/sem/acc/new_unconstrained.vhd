-- yavs-test: std=2008
-- expect: E1212 17:14 allocator needs a fully constrained subtype, `string` is unconstrained
-- expect: E1212 18:14 a record with an unconstrained element is not fully constrained
-- help: give the bounds
-- LRM 9.3.7
package p is
  procedure run;
end package;
package body p is
  type sp is access string;
  type rec_t is record s : string; end record;
  type rp is access rec_t;
  procedure run is
    variable p : sp;
    variable r : rp;
  begin
    p := new string;
    r := new rec_t;
    p := new string(1 to 3);
    r := new rec_t(s(1 to 3));
    r := new rec_t'(s => "abc");
  end procedure;
end package body;
