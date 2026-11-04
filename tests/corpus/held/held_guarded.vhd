-- difftest: top=held_guarded std=2008 stop=150ns ignore=order
-- ignore=order: reports of different processes and instances in one cycle.
-- Guarded assignments on plain, bus and register signals beside held drives; a disconnect time;
-- resolved signals with one and two sources.
library ieee; use ieee.std_logic_1164.all;
entity held_guarded_u is generic (g : natural); end;
architecture a of held_guarded_u is
  function res(v : std_ulogic_vector) return std_ulogic is
    variable r : std_ulogic := 'Z';
  begin
    for k in v'range loop if v(k) /= 'Z' then r := v(k); end if; end loop;
    return r;
  end function;
  subtype rl is res std_ulogic;
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal g1 : boolean := false;
  signal pl : std_logic := '0';
  signal bs : std_logic bus := 'Z';
  signal rg : std_logic register := 'U';
  signal ur : rl := 'Z';
  signal ur2 : rl := 'Z';
  signal two : std_logic := 'Z';
  signal one : std_logic := '0';
  disconnect bs : std_logic after 2 ns;
begin
  clk <= not clk after 5 ns when now < 130 ns;
  process (clk) begin if rising_edge(clk) then n <= n + 1; if n mod 4 = 1 then g1 <= not g1; end if; end if; end process;
  b : block (g1) begin
    pl <= guarded pl;
    bs <= guarded '1' when n mod 2 = 0 else bs;
    rg <= guarded rg;
  end block;
  process (clk) begin
    if rising_edge(clk) then
      if n < 0 or g > 1000 then report "n"; end if;
      ur <= ur;
      if n mod 3 = 0 then ur <= '1'; end if;
      ur2 <= ur2;
      two <= two;
      one <= one;
      if n mod 5 = 0 then one <= not one; end if;
    end if;
  end process;
  process (clk) begin
    if rising_edge(clk) then
      if n < 0 or g > 1000 then report "n"; end if;
      ur2 <= 'Z';
      two <= 'Z';
      if n mod 4 = 2 then two <= '0'; ur2 <= '1'; end if;
    end if;
  end process;
  process (pl, bs, rg, ur, ur2, two, one) begin
    report std_logic'image(pl) & std_logic'image(bs) & std_logic'image(rg) & std_ulogic'image(ur) & std_ulogic'image(ur2)
      & std_logic'image(two) & std_logic'image(one);
  end process;
end;
entity held_guarded is end;
architecture a of held_guarded is
begin
  u0 : entity work.held_guarded_u generic map (0);
  u1 : entity work.held_guarded_u generic map (1);
end;
