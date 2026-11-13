-- difftest: top=held_real_zero std=2008 ref=hand ignore=order
-- ref=hand: the first reference simulator makes an event of 0.0 to -0.0 (#1145); the second one
-- and this test do not. ignore=order: reports of different instances in one cycle.
-- Zero-delay drives of the value a REAL signal holds, and of its other zero, in an instance of
-- their own (so compiled code compares the drive inline and the kernel may skip it or compare a
-- whole record drive with its image): the zeros are a transaction, never an event.
library ieee; use ieee.std_logic_1164.all;
entity held_real_zero_u is generic (g : natural); end;
architecture a of held_real_zero_u is
  type rec is record r : real; l : std_logic; v : real_vector(0 to 1); end record;
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal s : real := 0.0;
  signal rv : real_vector(0 to 2) := (others => 0.0);
  signal rc : rec := (0.0, '0', (0.0, 0.0));
  signal nsc, nv, nr, nrc : natural := 0;
begin
  clk <= not clk after 5 ns when now < 400 ns;
  process (clk) begin if rising_edge(clk) then n <= n + 1; end if; end process;
  process (s) begin nsc <= nsc + 1; end process;
  process (rv) begin nv <= nv + 1; end process;
  process (rc.r) begin nr <= nr + 1; end process;
  process (rc) begin nrc <= nrc + 1; end process;
  process (clk)
    variable zero : real := real(g) * 0.0;
    variable neg : real := 0.0;
    variable x : real;
  begin
    if rising_edge(clk) then
      neg := -zero;                     -- negative zero, at run time
      case n mod 7 is
        when 0 | 1 | 4 => x := zero;
        when 2 | 3 => x := neg;
        when 5 => x := 1.0;
        when others => x := neg;
      end case;
      s <= x;
      rv <= (x, x, zero);
      rc <= (x, '0', (x, neg));
      rc <= (x, '0', (x, neg));
    end if;
  end process;
  process begin
    wait for 395 ns;
    report "g=" & integer'image(g) & " events s " & integer'image(nsc) & " rv " & integer'image(nv)
      & " rc.r " & integer'image(nr) & " rc " & integer'image(nrc) & " n " & integer'image(n);
    wait;
  end process;
end;
entity held_real_zero is end;
architecture a of held_real_zero is
begin
  u0 : entity work.held_real_zero_u generic map (0);
  u1 : entity work.held_real_zero_u generic map (1);
end;
