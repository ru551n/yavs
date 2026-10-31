-- difftest: top=driving_parts std=2008
-- 'driving and 'driving_value of signal parameters, record parts and composites over several
-- drivers of the process (LRM 16.2.4).
library ieee; use ieee.std_logic_1164.all;
entity driving_parts is end entity;
architecture a of driving_parts is
  type rec is record a, b : std_logic; v : std_logic_vector(1 downto 0); end record;
  signal r : rec;
  signal s : std_logic;
  signal w : std_logic_vector(3 downto 0);
  procedure ps (signal y : out std_logic) is begin
    report boolean'image(y'driving) ; y <= '1'; report to_string(y'driving_value); end procedure;
  procedure pv (signal y : inout std_logic_vector) is begin
    y <= y; report boolean'image(y'driving) & " " & to_string(y'driving_value); end procedure;
  procedure pr (signal y : inout rec) is begin
    y.a <= '1';
    report boolean'image(y.a'driving) & " " & to_string(y.a'driving_value) & " " & boolean'image(y'driving) & " " & to_string(y.v'driving_value); end procedure;
begin
  process begin
    r.b <= '0'; r.v <= "01"; 
    wait for 1 ns;
    report boolean'image(r.b'driving) & " " & to_string(r.b'driving_value);
    report boolean'image(r.v'driving) & " " & to_string(r.v'driving_value);
    report boolean'image(r'driving) & " " & to_string(r'driving_value.v);
    ps(s);
    pv(w);
    pr(r);
    wait for 1 ns;
    report to_string(r'driving_value.a);
    pv(w(1 downto 0));
    wait;
  end process;
end architecture;
