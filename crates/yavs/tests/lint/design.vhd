-- The rules about an architecture as a design. Run with every class on (see tests/lint.rs).
library work; -- expect: lint_005
library ieee;
use ieee.std_logic_1164.all;

entity leaf is
  port (a : in std_logic; b : out std_logic; c : in std_logic);
end entity;

architecture rtl of leaf is
begin
  b <= a and c;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity design is
  generic (n : positive := 2);
  port (
    clk, clk2, rst, rst2, a, d : in std_logic;
    y, z : out std_logic;
    bus_out : out std_logic_vector(3 downto 0)
  );
end entity;

architecture rtl of design is
  component leaf is -- expect: lint_750
    port (a : in std_logic; b : in std_logic; spare : in std_logic);
  end component;
  component absent is
    port (a : in std_logic);
  end component;

  type state_t is (idle, running, stuck, aborted); -- expect: lint_710
  signal state : state_t := idle;
  type mode_t is (m0, m1, m2); -- expect: lint_710, lint_710
  signal mode : mode_t;

  signal two : std_logic; -- no finding of its own: lint_601 below
  signal grant, request : std_logic;
  signal enable : std_logic; -- expect: lint_730
  signal tied : std_logic := '1';
  signal never_used : std_logic; -- expect: lint_004
  constant in_a_range : positive := 3;
  signal bits : std_logic_vector(in_a_range downto 0); -- expect: lint_004
  signal q_a, flag_a, in_a, in_b, sync_b : std_logic;
  signal counter_r : std_logic;
  signal counter : std_logic;
  signal latched : std_logic;
  signal lat_ok : std_logic;
  signal part : std_logic_vector(1 downto 0);
  signal mix : std_logic;
  signal both_q, both_r : std_logic;
  signal half_r : std_logic;
begin
  two <= a; -- expect: lint_601
  two <= d;

  -- Two bits of one signal are two signals.
  part(0) <= a;
  part(1) <= d;
  bus_out(1 downto 0) <= part;
  bus_out(3 downto 2) <= "00";

  -- Exclusive alternatives never exist together.
  which : if n = 1 generate
    mix <= a;
  else generate
    mix <= d;
  end generate;

  grant <= request and a; -- expect: lint_720
  request <= grant or a;

  y <= enable and tied and two and mix;

  u1 : leaf port map (a => a, b => a, spare => a);

  missing : process (a) is -- has a read of d that the list lacks
  begin
    z <= a and d; -- expect: lint_001
  end process;

  unread : process (a, d) is -- expect: lint_002
  begin
    latched <= a;
  end process;

  latch : process (a, d) is
  begin
    if a = '1' then
      lat_ok <= d; -- expect: lint_600
    end if;
  end process;

  fsm : process (clk) is
  begin
    if rising_edge(clk) then
      case state is
        when idle => state <= running;
        when running =>
          if a = '1' then
            state <= stuck;
          else
            state <= idle;
          end if;
        when stuck => state <= stuck; -- expect: lint_711
        when aborted => state <= idle;
      end case;
      case mode is
        when m0 => null;
        when m1 => null;
        when m2 => null;
        when others => null; -- expect: lint_712
      end case;
      case mode is
        when m0 => null;
        when others => null; -- expect: lint_713
      end case;
    end if;
  end process;

  mode_p : process (clk) is
  begin
    if rising_edge(clk) then
      mode <= m0;
      counter <= a;
      counter_r <= a;
    end if;
  end process;

  -- `in_a` is reset by rst and registered on clk; `q_a` is read on clk2 in logic.
  dom_a : process (clk, rst) is
  begin
    if rst = '1' then
      q_a <= '0';
      in_a <= '0';
      flag_a <= '0';
    elsif rising_edge(clk) then
      q_a <= a;
      in_a <= d;
      flag_a <= a;
      both_q <= d; -- expect: lint_702
    end if;
  end process;

  dom_b : process (clk2, rst2) is
  begin
    if rst2 = '1' then
      in_b <= '0';
      sync_b <= '0';
      both_r <= '0';
    elsif rising_edge(clk2) then
      in_b <= flag_a; -- a capture and nothing else: no crossing
      sync_b <= in_b;
      both_r <= q_a and d; -- expect: lint_700, lint_701
    end if;
  end process;

  on_fall : process (clk) is -- expect: lint_703
  begin
    if falling_edge(clk) then
      half_r <= a;
    end if;
  end process;
end architecture;

package recursive is
  function fact (n : integer) return integer;
  function to_text (n : integer) return string;
  function to_text (n : boolean) return string;
end package;

package body recursive is
  function fact (n : integer) return integer is
  begin
    if n <= 1 then
      return 1;
    end if;
    return n * fact(n - 1); -- expect: lint_760
  end function;

  -- Another subprogram of the name is no recursion.
  function to_text (n : integer) return string is
  begin
    return to_text(n > 0);
  end function;

  function to_text (n : boolean) return string is
  begin
    return boolean'image(n);
  end function;
end package body;
