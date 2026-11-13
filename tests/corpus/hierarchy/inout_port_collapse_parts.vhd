-- difftest: top=inout_port_collapse_parts std=2008 ignore=order
-- `inout` ports on parts and through components: a vector formal on a slice of a wider actual
-- (`w(5 downto 2)`), driven inside as a whole and element by element while the parent drives
-- the same and the neighbouring elements, with a default equal to the actual's initial value
-- ('Z', not 'U'); a component port whose default differs from the entity port's (the entity's
-- driver starts at the entity port's default).
-- ignore=order: the readers of the levels report in the same delta.
library ieee;
use ieee.std_logic_1164.all;

entity iopp_vec is
  port (v : inout std_logic_vector(3 downto 0) := "ZZZZ");
end;
architecture t of iopp_vec is
begin
  drv : process
  begin
    v <= "1Z0Z";
    wait for 2 ns;
    v(0) <= '1';
    wait for 2 ns;
    v(2 downto 1) <= "HL";
    wait for 2 ns;
    v <= "ZZZZ";
    wait;
  end process;
  rd : process (v)
  begin
    -- 'last_value before the first event differs between the references (U, Z), and that of
    -- a whole vector (catalogue `last-value-composite`): elements only, after the first event.
    if now > 0 ns then
      report "vec v=" & to_string(v) & " v(0)'last_value=" & to_string(v(0)'last_value)
        & " v(2)'last_value=" & to_string(v(2)'last_value) & " v(3)'event="
        & to_string(v(3)'event);
    else
      report "vec v=" & to_string(v);
    end if;
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;

entity iopp_ent is
  port (c : inout std_logic := '0');
end;
architecture t of iopp_ent is
begin
  drv : process
  begin
    wait for 3 ns;
    c <= 'Z';
    wait for 2 ns;
    c <= 'H';
    wait;
  end process;
  rd : process (c)
  begin
    report "ent c=" & to_string(c);
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;

entity iopp_mid is
  port (m : inout std_logic);
end;
architecture t of iopp_mid is
  component iopp_ent is
    port (c : inout std_logic := '1');
  end component;
begin
  u : iopp_ent port map (c => m);
end;

library ieee;
use ieee.std_logic_1164.all;

entity inout_port_collapse_parts is end;
architecture t of inout_port_collapse_parts is
  signal w : std_logic_vector(7 downto 0) := (others => 'Z');
  signal x : std_logic;
begin
  uv : entity work.iopp_vec port map (v => w(5 downto 2));
  um : entity work.iopp_mid port map (m => x);

  drv_w : process
  begin
    w(7 downto 4) <= "0ZZ1";
    w(1 downto 0) <= "LH";
    wait for 1 ns;
    w(7 downto 4) <= "ZZZZ";
    wait for 4 ns;
    w(5) <= '0';
    wait;
  end process;
  drv_x : x <= 'Z', 'L' after 4 ns;

  rd : process (w, x)
  begin
    report "top w=" & to_string(w) & " x=" & to_string(x);
  end process;

  check : process
  begin
    wait for 9 ns;
    assert w = "ZZ0ZZZLH" report "w=" & to_string(w) severity failure;
    assert x = 'W' report "x=" & to_string(x) severity failure;
    report "done";
    wait;
  end process;
end;
