-- lint_740 and lint_741: the width of a vector against another.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity width is
  generic (n : positive := 8);
  port (
    wide   : in  std_logic_vector(15 downto 0);
    narrow : out std_logic_vector(7 downto 0);
    free   : in  std_logic_vector;
    by_g   : out std_logic_vector(n - 1 downto 0);
    q      : out bit;
    clk    : in  std_logic
  );
end entity;

architecture rtl of width is
  subtype byte is std_logic_vector(7 downto 0);
  signal a, b : byte;
  signal c : std_logic_vector(0 to 3);
  signal u : unsigned(3 downto 0);
  constant lit : std_logic_vector(2 downto 0) := "000";
begin
  narrow <= wide; -- expect: lint_740
  narrow <= a;
  narrow <= wide(7 downto 0);
  narrow <= wide(8 downto 0); -- expect: lint_740
  a <= x"ff";
  -- Operators and calls can give any length: not judged.
  narrow <= not a;
  narrow <= a(3 downto 0) & b(3 downto 0);
  narrow <= std_logic_vector(u) & "0000";
  -- A generic decides, or the port takes its bounds from the actual.
  by_g <= a;
  narrow <= free;
  c <= free;
  -- A conditional assignment is measured per value.
  a <= b when clk = '1' else wide; -- expect: lint_740

  clocked : process (clk) is
    variable v : std_logic_vector(3 downto 0);
    variable text_value : string(1 to 5);
  begin
    if rising_edge(clk) then
      a <= wide; -- expect: lint_740
      v := a; -- expect: lint_740
      v := c;
      if c = "000" then -- expect: lint_741
        null;
      end if;
      if c /= a then -- expect: lint_741
        null;
      end if;
      if a = wide(7 downto 0) then
        null;
      end if;
      if a = lit then -- expect: lint_741
        null;
      end if;
      -- An overloaded `=` resizes its operands: legal and useful.
      if u = "000" then
        null;
      end if;
      -- A call has no certain width.
      if a = std_logic_vector(u) then
        null;
      end if;
      if text_value = "abc" then -- expect: lint_741
        null;
      end if;
    end if;
  end process;

  q <= '1' when a = "000" else '0'; -- expect: lint_741
end architecture;
