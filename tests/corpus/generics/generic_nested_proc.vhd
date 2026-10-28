-- difftest: top=generic_nested_proc ref=hand relaxed
-- ref=hand: only a report, no reference-specific output.
-- A procedure declared inside a generic subprogram and called from its body gets its body in
-- the instance too (#621): the call must not be "body not analyzed".
package generic_nested_pkg is
  type record_t is record
    a : integer;
  end record;
  procedure monitor generic (type t);
  procedure monitor generic (type t) is
    procedure sub is
    begin
      report "sub called";
    end procedure;
  begin
    sub;
  end procedure;
end package;
use work.generic_nested_pkg.all;
entity generic_nested_proc is end;
architecture a of generic_nested_proc is
  procedure m is new monitor generic map (t => record_t);
begin
  process begin
    m;
    wait;
  end process;
end;
