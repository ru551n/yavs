-- A record drive whose every leaf changes at every clock: the compare spares nothing and the image
-- is written all the same, so the review lets the image go after a thousand scatters.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity rec_cmp_drop is generic (CYC : natural := 1400); end;
architecture a of rec_cmp_drop is
  type rec is record
    a : std_ulogic;
    b : unsigned(15 downto 0);
    i : integer;
    c : std_ulogic;
    n : unsigned(7 downto 0);
  end record;
  signal clk : std_ulogic := '0';
  signal r : rec := ('0', x"0000", 0, '1', x"00");
begin
  reg : process (clk)
    variable v : rec := ('0', x"0000", 0, '1', x"00");
  begin
    if rising_edge(clk) then
      v := (not v.a, v.b + 1, v.i + 1, not v.c, v.n + 1);
      r <= v;
    end if;
  end process;
  main : process
  begin
    for i in 1 to CYC loop
      clk <= '1'; wait for 5 ns;
      clk <= '0'; wait for 5 ns;
    end loop;
    report "i=" & integer'image(r.i) & " n=" & integer'image(to_integer(r.n));
    wait;
  end process;
end;
