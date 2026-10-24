-- A check whose condition and message convert the same vector twice (to_integer of a
-- concatenation): the second call is made again only when the first returned 0 (#520), which
-- needs the two calls' operands (planes and bounds) to be recognised as the same.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
package chk is
  type status_t is protected
    procedure assertion(condition : boolean; message : string);
    impure function failed return natural;
  end protected;
  procedure check(cond : boolean; message : string);
  impure function failures return natural;
end package;
package body chk is
  type status_t is protected body
    variable failed_n : natural := 0;
    procedure assertion(condition : boolean; message : string) is
    begin
      if not condition then failed_n := failed_n + 1; end if;
    end procedure;
    impure function failed return natural is begin return failed_n; end function;
  end protected body;
  shared variable st : status_t;
  procedure check(cond : boolean; message : string) is
  begin
    st.assertion(cond, message);
  end procedure;
  impure function failures return natural is begin return st.failed; end function;
end package body;

library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all; use work.chk.all;
entity ieee_share is end;
architecture a of ieee_share is
  signal c : std_logic_vector(3 downto 0) := "0101";
  signal s : std_logic_vector(3 downto 0) := "0011";
begin
  process
  begin
    for k in 1 to 100 loop
      check(to_integer(unsigned(c & s)) = 84, "got " & integer'image(to_integer(unsigned(c & s))));
    end loop;
    report "n=" & integer'image(failures);
    wait;
  end process;
end;
