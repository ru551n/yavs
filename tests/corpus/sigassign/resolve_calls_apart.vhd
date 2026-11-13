-- difftest: top=resolve_calls_apart ignore=order,path
-- A resolution function that reports each call, on vectors of a resolved subtype whose
-- elements 0 and 3 get their transactions apart, in the order 3, 0, 3, 0 (each driver of `e`
-- drives both elements but assigns one; `f` is driven through out ports of one element each):
-- each element is resolved once per update with all its sources (LRM 14.7.3.2). The order of
-- the calls of one update is not compared (nor the paths, so the reports of the function sort by
-- text): the LRM leaves it open and the references differ.
package resolve_calls_apart_pkg is
  type int_v is array (natural range <>) of integer;
  function res_max (v : int_v) return integer;
  subtype rint is res_max integer;
  type rint_vec is array (natural range <>) of rint;
end;
package body resolve_calls_apart_pkg is
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
entity resolve_calls_apart_one is
  port (clk : in bit; k : in integer; o : out integer := 0);
end;
architecture a of resolve_calls_apart_one is
begin
  process (clk)
    variable n : integer := 0;
  begin
    if clk'event and clk = '1' then n := n + 1; o <= n * 1000 + k; end if;
  end process;
end;
use work.resolve_calls_apart_pkg.all;
entity resolve_calls_apart is end;
architecture a of resolve_calls_apart is
  signal clk : bit := '0';
  signal e, f : rint_vec(0 to 3) := (others => 0);
begin
  process begin
    for i in 1 to 2 loop clk <= '1'; wait for 5 ns; clk <= '0'; wait for 5 ns; end loop; wait;
  end process;
  g : for k in 0 to 3 generate
    process (clk)
      variable n : integer := 0;
    begin
      if clk'event and clk = '1' then
        n := n + 1;
        if k mod 2 = 0 then e(3) <= n * 100 + k; else e(0) <= n * 10 + k; end if;
      end if;
    end process;
    u : entity work.resolve_calls_apart_one
      port map (clk => clk, k => k, o => f(3 * ((k + 1) mod 2)));
  end generate;
  process (e, f) begin
    report "e=" & integer'image(e(0)) & "," & integer'image(e(3)) & " f=" & integer'image(f(0))
      & "," & integer'image(f(3));
  end process;
end;
