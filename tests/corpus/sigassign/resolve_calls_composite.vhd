-- difftest: top=resolve_calls_composite ref=hand
-- Resolution functions that report each call, on a record and an array resolved as a whole,
-- three drivers each, all assigning every clock: each is resolved once per update with all its
-- sources (LRM 14.7.3.2), not once per driver.
-- ref=hand: the reference simulator calls the function of the record once per field (catalogue
-- `record-resolution-calls`) and crashes on the array resolved as a whole (`array-resolved-whole`);
-- the expected output is the LRM's, which another simulator gives.
library ieee; use ieee.std_logic_1164.all;
package resolve_calls_composite_pkg is
  type rec is record a : integer; b : std_ulogic; end record;
  type rec_v is array (natural range <>) of rec;
  function res_rec (v : rec_v) return rec;
  subtype rrec is res_rec rec;
  type pair is array (0 to 1) of integer;
  type pair_v is array (natural range <>) of pair;
  function res_pair (v : pair_v) return pair;
  subtype rpair is res_pair pair;
end;
package body resolve_calls_composite_pkg is
  function res_rec (v : rec_v) return rec is
    variable r : rec := (0, '0');
  begin
    for i in v'range loop r.a := r.a + v(i).a; if v(i).b = '1' then r.b := '1'; end if; end loop;
    if r.a /= 0 then
      report "res_rec of " & integer'image(v'length) & " sources: " & integer'image(r.a) & " "
        & std_ulogic'image(r.b);
    end if;
    return r;
  end;
  function res_pair (v : pair_v) return pair is
    variable p : pair := (0, 0);
  begin
    for i in v'range loop p(0) := p(0) + v(i)(0); p(1) := p(1) + v(i)(1); end loop;
    if p(0) /= 0 then
      report "res_pair of " & integer'image(v'length) & " sources: " & integer'image(p(0)) & " "
        & integer'image(p(1));
    end if;
    return p;
  end;
end;
library ieee; use ieee.std_logic_1164.all; use work.resolve_calls_composite_pkg.all;
entity resolve_calls_composite is end;
architecture a of resolve_calls_composite is
  signal clk : std_logic := '0';
  signal rr : rrec := (0, '0');
  signal rp : rpair := (0, 0);
begin
  process begin
    for i in 1 to 3 loop clk <= '1'; wait for 5 ns; clk <= '0'; wait for 5 ns; end loop; wait;
  end process;
  g : for k in 0 to 2 generate
    process (clk)
      variable n : integer := 0;
    begin
      if rising_edge(clk) then
        n := n + 1;
        if k = 1 then rr <= (n * 10, '1'); else rr <= (k, '0'); end if;
        rp <= (n, k);
      end if;
    end process;
  end generate;
  process (rr, rp) begin
    report "rr=" & integer'image(rr.a) & " rp=" & integer'image(rp(0)) & "," & integer'image(rp(1));
  end process;
end;
