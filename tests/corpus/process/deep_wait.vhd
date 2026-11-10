-- difftest: top=deep_wait
-- A process that waits inside a chain of procedures, as verification components do: `DEPTH` 0
-- waits in the process itself, 1, 3 and 6 inside that many nested procedure calls. Each wake
-- resumes the whole frame stack. The innermost procedure's loop is a `wait until`, which
-- `--tier=auto` does not count as a compute loop (`wait_loops`). Every tier must print the same.
-- `ONLY` runs one waiter alone.
library ieee;
use ieee.std_logic_1164.all;

package deep_wait_pkg is
  procedure w1(signal s : in std_logic; n : inout natural);
  procedure w2(signal s : in std_logic; n : inout natural);
  procedure w3(signal s : in std_logic; n : inout natural);
  procedure w4(signal s : in std_logic; n : inout natural);
  procedure w5(signal s : in std_logic; n : inout natural);
  procedure w6(signal s : in std_logic; n : inout natural);
end package;

package body deep_wait_pkg is
  -- The shape of a toggle wait: one wait of three, chosen by the value now.
  procedure w1(signal s : in std_logic; n : inout natural) is
  begin
    if s = '0' then
      wait until s = '1';
    elsif s = '1' then
      wait until s = '0';
    else
      wait until s = '0' or s = '1';
    end if;
    n := n + 1;
  end procedure;

  procedure w2(signal s : in std_logic; n : inout natural) is
  begin
    w1(s, n);
  end procedure;

  procedure w3(signal s : in std_logic; n : inout natural) is
  begin
    w2(s, n);
  end procedure;

  procedure w4(signal s : in std_logic; n : inout natural) is
  begin
    w3(s, n);
  end procedure;

  procedure w5(signal s : in std_logic; n : inout natural) is
  begin
    w4(s, n);
  end procedure;

  procedure w6(signal s : in std_logic; n : inout natural) is
  begin
    w5(s, n);
  end procedure;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.deep_wait_pkg.all;

entity deep_wait is
  generic (N : natural := 300; ONLY : integer := -1);
end entity;

architecture a of deep_wait is
  type sl_array is array (0 to 3) of std_logic;
  signal s : sl_array := (others => '0');
  type depth_array is array (0 to 3) of natural;
  constant DEPTH : depth_array := (0, 1, 3, 6);
begin
  toggle : process
  begin
    for i in 1 to N loop
      wait for 1 ns;
      s <= (not s(0), not s(1), not s(2), not s(3));
    end loop;
    wait;
  end process;

  waiters : for k in 0 to 3 generate
    waiter : process
      variable woke : natural := 0;
    begin
      if ONLY < 0 or ONLY = k then
        while woke < N loop
          case DEPTH(k) is
            when 0 =>
              if s(k) = '0' then
                wait until s(k) = '1';
              else
                wait until s(k) = '0';
              end if;
              woke := woke + 1;
            when 1 => w1(s(k), woke);
            when 3 => w3(s(k), woke);
            when others => w6(s(k), woke);
          end case;
        end loop;
        report "depth " & integer'image(DEPTH(k)) & " woke " & integer'image(woke);
        assert woke = N severity failure;
      end if;
      wait;
    end process;
  end generate;
end architecture;
