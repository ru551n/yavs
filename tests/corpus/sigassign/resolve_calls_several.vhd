-- difftest: top=resolve_calls_several
-- A resolution function that reports each call, on signals updated in the same cycles: the
-- elements of a vector of a resolved subtype, whose drivers cover different elements, and a
-- resolved integer. Each element and the integer are resolved once per update with all their
-- sources (LRM 14.7.3.2), not once per driver.
package resolve_calls_several_pkg is
  type int_v is array (natural range <>) of integer;
  function res_max (v : int_v) return integer;
  subtype rint is res_max integer;
  type rint_vec is array (natural range <>) of rint;
end;
package body resolve_calls_several_pkg is
  function res_max (v : int_v) return integer is
    variable m : integer := 0;
  begin
    for i in v'range loop if v(i) > m then m := v(i); end if; end loop;
    if m /= 0 then
      report "res_max of " & integer'image(v'length) & " sources: " & integer'image(m);
    end if;
    return m;
  end;
end;
use work.resolve_calls_several_pkg.all;
entity resolve_calls_several is end;
architecture a of resolve_calls_several is
  signal clk : bit := '0';
  signal e : rint_vec(0 to 1) := (0, 0);
  signal s2 : rint := 0;
begin
  process begin
    for i in 1 to 3 loop clk <= '1'; wait for 5 ns; clk <= '0'; wait for 5 ns; end loop; wait;
  end process;
  -- Driver k of `e`: element 0 for k = 0, both for k = 1, element 1 for k = 2.
  g : for k in 0 to 2 generate
    process (clk)
      variable n : integer := 0;
    begin
      if clk'event and clk = '1' then
        n := n + 1;
        if k = 0 then e(0) <= n; elsif k = 1 then e <= (n * 10, n * 20); else e(1) <= n; end if;
        s2 <= n * 100 + k;
      end if;
    end process;
  end generate;
  process (e, s2) begin
    report "e=" & integer'image(e(0)) & "," & integer'image(e(1)) & " s2=" & integer'image(s2);
  end process;
end;
