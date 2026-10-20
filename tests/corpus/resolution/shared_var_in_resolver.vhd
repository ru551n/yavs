-- difftest: top=shared_var_in_resolver relaxed
-- A resolution function that reaches a shared variable of a non-protected type (accepted with
-- the relaxed switch) through a procedure: for a record resolved as a whole and for a scalar
-- signal resolved per element. The calls are counted, so the counts also show that the
-- function runs once per resolution of each signal, in any thread count.
package shared_var_in_resolver_p is
  type rec is record a : integer; end record;
  type rec_vec is array (natural range <>) of rec;
  function resolved(v : rec_vec) return rec;
  subtype rrec is resolved rec;
  function res_int(v : integer_vector) return integer;
  subtype rint is res_int integer;
  shared variable cnt_rec : integer := 0;
  shared variable cnt_int : integer := 0;
  procedure bump_rec;
  procedure bump_int;
end package;
package body shared_var_in_resolver_p is
  procedure bump_rec is begin cnt_rec := cnt_rec + 1; end procedure;
  procedure bump_int is begin cnt_int := cnt_int + 1; end procedure;
  function resolved(v : rec_vec) return rec is
    variable r : rec := v(v'low);
  begin
    if v'length = 1 then return v(v'low); end if;
    for i in v'range loop
      if v(i).a /= -1 then r := v(i); end if;
    end loop;
    bump_rec;
    return r;
  end function;
  function res_int(v : integer_vector) return integer is
    variable r : integer := v(v'low);
  begin
    if v'length = 1 then return v(v'low); end if;
    for i in v'range loop
      if v(i) /= -1 then r := v(i); end if;
    end loop;
    bump_int;
    return r;
  end function;
end package body;
use work.shared_var_in_resolver_p.all;
entity shared_var_in_resolver is end;
architecture a of shared_var_in_resolver is
  signal s : rrec := (a => -1);
  signal i : rint := -1;
begin
  p1 : process begin s <= (a => 1); i <= 1; wait; end process;
  p2 : process begin wait for 1 ns; s <= (a => 2); i <= 2; wait; end process;
  p3 : process
  begin
    wait for 5 ns;
    report "rec " & integer'image(cnt_rec) & " int " & integer'image(cnt_int);
    report "s " & integer'image(s.a) & " i " & integer'image(i);
    wait;
  end process;
end;
