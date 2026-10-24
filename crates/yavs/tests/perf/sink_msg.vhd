library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package chk is
  type status_t is protected
    procedure assertion(condition : boolean; message : string := "");
    impure function failed return natural;
  end protected;
  procedure check(cond : boolean; message : string := "");
  impure function failures return natural;
end package;

package body chk is
  type status_t is protected body
    variable total  : natural := 0;
    variable failed_n : natural := 0;
    procedure fail(message : string := "") is
    begin
      if message'length > 0 then
        report message severity warning;
      end if;
    end procedure;
    procedure assertion(condition : boolean; message : string := "") is
    begin
      total := total + 1;
      if not condition then
        fail(message);
        failed_n := failed_n + 1;
      end if;
    end procedure;
    impure function failed return natural is
    begin
      return failed_n;
    end function;
  end protected body;
  shared variable st : status_t;
  procedure check(cond : boolean; message : string := "") is
  begin
    st.assertion(cond, message);
  end procedure;
  impure function failures return natural is
  begin
    return st.failed;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.chk.all;

entity sink_msg is end;
architecture a of sink_msg is
  type color is (red, green, blue);
  signal v : std_logic_vector(7 downto 0) := x"2a";
begin
  process
    variable acc : integer := 0;
  begin
    -- The message of a check is built for every call and read by the failing ones only.
    for i in 0 to 99 loop
      for c in color loop
        check(i mod 10 /= 0 or c /= blue,
              "bad[" & color'image(c) & "] i=" & integer'image(i) & " v=" &
              integer'image(to_integer(unsigned(v))));
        acc := acc + i;
      end loop;
    end loop;
    report "failed=" & integer'image(failures) & " acc=" & integer'image(acc);
    wait;
  end process;
end;
