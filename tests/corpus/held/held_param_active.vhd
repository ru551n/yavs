-- difftest: top=held_param_active std=2008 stop=120ns ignore=order
-- ignore=order: reports of different processes and instances in one cycle.
-- 'active and 'last_active read where the signal is not named: a parameter in a package body, an
-- alias, an external name. A held drive in another process still counts.
library ieee; use ieee.std_logic_1164.all;
package held_pa is
  procedure show(signal x : in std_logic; tag : string);
end package;
package body held_pa is
  procedure show(signal x : in std_logic; tag : string) is
  begin
    report tag & " " & boolean'image(x'active) & " " & time'image(x'last_active);
  end procedure;
end package body;
library ieee; use ieee.std_logic_1164.all; use work.held_pa.all;
entity held_sub is end;
architecture a of held_sub is
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal s : std_logic := '0';
begin
  clk <= not clk after 7 ns when now < 80 ns;
  process (clk) begin if rising_edge(clk) then n <= n + 1; s <= s; end if; end process;
end;
library ieee; use ieee.std_logic_1164.all; use work.held_pa.all;
entity held_param_active_u is generic (g : natural); end;
architecture a of held_param_active_u is
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal s, p : std_logic := '0';
  alias al : std_logic is p;
begin
  clk <= not clk after 5 ns when now < 100 ns;
  u : entity work.held_sub;
  process (clk) begin if rising_edge(clk) then n <= n + 1; end if; end process;
  process (clk) begin
    if rising_edge(clk) then
      if n >= 0 or g > 1000 then
        s <= s;
        p <= p;
      end if;
    end if;
  end process;
  process
    alias ext is <<signal u.s : std_logic>>;
  begin
    wait until clk = '1';
    wait for 0 ns;
    wait for 0 ns;
    show(s, "param");
    report "alias " & boolean'image(al'active) & " " & time'image(al'last_active);
    report "external " & boolean'image(ext'active) & " " & time'image(ext'last_active);
  end process;
end;
entity held_param_active is end;
architecture a of held_param_active is
begin
  u0 : entity work.held_param_active_u generic map (0);
  u1 : entity work.held_param_active_u generic map (1);
end;
