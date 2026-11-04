-- difftest: top=held_transaction std=2008 stop=120ns ignore=order
-- A held drive is a transaction: 'transaction toggles, 'quiet falls, 'stable does not.
-- ignore=order: reports of different processes in one cycle.
library ieee; use ieee.std_logic_1164.all;
entity held_transaction_u is generic (g : natural); end;
architecture a of held_transaction_u is
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal s, q : std_logic := '0';
  signal v : std_logic_vector(7 downto 0) := x"00";
  signal i, j : integer := 0;
  type pair is record a, b : std_logic; end record;
  signal r : pair := ('0', '0');
begin
  clk <= not clk after 5 ns when now < 100 ns;
  process (clk) begin if rising_edge(clk) then n <= n + 1; end if; end process;
  process (clk) begin
    if rising_edge(clk) then
      if n >= 0 or g > 1000 then
        s <= s;
        q <= q;
        v(3) <= v(3);
        i <= i;
        j <= j;
        r.a <= r.a;
        r.b <= r.b;
      end if;
    end if;
  end process;
  process begin wait on s'transaction; report "s tr"; end process;
  process begin wait on v(3)'transaction; report "v3 tr"; end process;
  process begin wait on i'transaction; report "i tr"; end process;
  process begin wait on r'transaction; report "r tr"; end process;
  process (q'quiet, j'stable(0 ns)) begin report "q quiet " & boolean'image(q'quiet) & " j stable " & boolean'image(j'stable(0 ns)); end process;
end;
entity held_transaction is end;
architecture a of held_transaction is
begin
  u0 : entity work.held_transaction_u generic map (0);
  u1 : entity work.held_transaction_u generic map (1);
end;
