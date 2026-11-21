-- `--init-x`: metavalue literals the design below the top assigns: `(others => 'X')` in a `case`
-- default, `'-'` in the last `else`, a `'U'` anywhere; `'-'` masks outside a default branch (one in
-- a `then` branch inside an `else`) and the testbench's own literals stay as written. Widths 1, 63, 64, 65 and 128.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity ix_dut is
  generic (W : positive);
  port (
    clk : in std_logic;
    sel : in std_logic_vector(1 downto 0);
    d : in std_logic_vector(W - 1 downto 0);
    y, q : out std_logic_vector(W - 1 downto 0);
    b, u : out std_logic;
    m : out boolean);
end entity;

architecture rtl of ix_dut is
  signal mask, mm : std_logic_vector(W - 1 downto 0);
begin
  process (all) begin
    case sel is
      when "00" => y <= d;
      when "01" => y <= not d;
      when others => y <= (others => 'X');
    end case;
  end process;
  b <= d(0) when sel = "00" else '-';
  u <= 'U' when sel = "10" else d(0);
  mask <= (others => '-');
  -- A mask in a `then` branch inside an `else`: no default branch either.
  process (all) begin
    if sel = "11" then
      mm <= d;
    else
      if sel /= "11" then
        mm <= (others => '-');
      else
        mm <= d;
      end if;
    end if;
  end process;
  m <= std_match(d, mask) and std_match(d, mm);
  process (clk)
    variable t : std_logic_vector(W - 1 downto 0);
  begin
    if rising_edge(clk) then
      if sel = "00" then
        t := d;
      else
        t := (others => 'W');
      end if;
      q <= t;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity initx is
end entity;

architecture tb of initx is
  type widths_t is array (0 to 4) of positive;
  constant WIDTHS : widths_t := (1, 63, 64, 65, 128);
  signal clk : std_logic := '0';
  signal sel : std_logic_vector(1 downto 0) := "00";
  signal tbx : std_logic := 'X';
begin
  clk <= not clk after 5 ns;
  g : for i in WIDTHS'range generate
    constant W : positive := WIDTHS(i);
    signal d : std_logic_vector(W - 1 downto 0) := (others => '1');
    signal y, q : std_logic_vector(W - 1 downto 0);
    signal b, u : std_logic;
    signal m : boolean;
  begin
    dut : entity work.ix_dut generic map (W) port map (clk, sel, d, y, q, b, u, m);
    process begin
      for k in 0 to 2 loop
        wait until rising_edge(clk);
        wait for 1 ns;
        report "W=" & integer'image(W) & " sel=" & to_string(sel) & " y=" & to_hstring(y)
          & " q=" & to_hstring(q) & " b=" & std_logic'image(b) & " u=" & std_logic'image(u)
          & " m=" & boolean'image(m) & " tb=" & std_logic'image(tbx);
      end loop;
      wait;
    end process;
  end generate;
  process begin
    wait until rising_edge(clk);
    wait for 2 ns;
    sel <= "10";
    tbx <= 'X';
    wait until rising_edge(clk);
    wait for 2 ns;
    sel <= "01";
    wait for 40 ns;
    std.env.finish;
  end process;
end architecture;
