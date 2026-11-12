-- difftest: top=copy_resume_drive
-- Processes of generate lanes that wait in a loop and drive their own element, slice or record field
-- (#1137). A process switches to its specialised copy between two activations (after 1024 of them by
-- default); the copy reads the generate parameter at run time, and its first activation must drive
-- this lane's part, not lane 0's. Each process runs about 1500 activations, past the switch.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity copy_resume_drive is
end entity;

architecture t of copy_resume_drive is
  constant N : positive := 4;
  type rec_t is record
    a : std_logic;
    n : integer;
  end record;
  type rec_arr_t is array (0 to N - 1) of rec_t;
  type int_arr_t is array (0 to N - 1) of integer;
  signal clk  : std_logic := '0';
  signal done : boolean := false;
  signal v    : std_logic_vector(0 to N - 1) := (others => '0');
  signal q    : std_logic_vector(0 to N - 1) := (others => '0');
  signal s    : std_logic_vector(0 to 2 * N - 1) := (others => '0');
  signal r    : rec_arr_t := (others => ('0', 0));
  signal hits : int_arr_t := (others => 0);
begin
  clk <= not clk after 5 ns when not done;

  lanes : for l in 0 to N - 1 generate
    process (clk) begin
      if rising_edge(clk) then
        v(l) <= not v(l);
      end if;
    end process;

    -- The issue's shape: one element, indexed by the parameter.
    process begin
      loop
        wait until v(l) = '1';
        q(l) <= not q(l);
      end loop;
    end process;

    -- An index computed from the parameter, a slice target and a variable across the waits.
    process
      variable k : natural := 0;
    begin
      loop
        wait until v((l + 1) mod N) = '1';
        k := k + 1;
        s(2 * l to 2 * l + 1) <= std_logic_vector(to_unsigned(k mod 4, 2));
        hits(l) <= k;
      end loop;
    end process;

    -- A field of a record element, read and driven.
    process begin
      wait on v(N - 1 - l);
      r(N - 1 - l).n <= r(N - 1 - l).n + 1;
      r(N - 1 - l).a <= v(N - 1 - l);
    end process;
  end generate;

  process begin
    wait for 15 us;
    done <= true;
    wait for 1 ns;
    report "q=" & to_string(q) & " s=" & to_string(s);
    for i in 0 to N - 1 loop
      report "lane " & integer'image(i) & ": hits " & integer'image(hits(i))
        & " n " & integer'image(r(i).n) & " a " & std_logic'image(r(i).a);
    end loop;
    wait;
  end process;
end architecture;
