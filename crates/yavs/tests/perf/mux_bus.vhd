-- A bus switch of conditional assignments (a record, a vector, a scalar, a literal arm) and a
-- relation, the select changed 40 times: muxes and gates the kernel computes itself (#786),
-- with no process code.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity mux_bus is end;
architecture a of mux_bus is
  type req is record
    addr : std_logic_vector(31 downto 0);
    data : std_logic_vector(31 downto 0);
    stb  : std_logic;
    n    : integer;
  end record;
  signal sel, ack : std_logic := '0';
  signal ra, rb, rx : req := (addr => (others => '0'), data => (others => '0'), stb => '0', n => 0);
  signal rd, rdx : std_logic_vector(31 downto 0) := (others => '0');
  signal ackx, en : std_logic;
  signal hit : boolean;
  signal low : std_logic_vector(31 downto 0);
begin
  rx <= ra when sel = '0' else rb;
  ackx <= ack when sel = '0' else '0';
  rdx <= rd when ackx = '1' else (others => '0');
  hit <= ra.addr = rb.addr;
  en <= ackx and '1';
  low <= rdx and x"0000ffff";

  process
    variable s : integer := 0;
  begin
    for k in 1 to 40 loop
      sel <= std_logic'val(2 + k mod 2);
      ack <= std_logic'val(2 + k / 2 mod 2);
      ra <= (addr => std_logic_vector(to_unsigned(k * 37, 32)), data => x"00000001", stb => '1', n => k);
      rb <= (addr => std_logic_vector(to_unsigned(k * 37 * (k mod 3), 32)), data => x"00000002", stb => '0', n => -k);
      rd <= std_logic_vector(to_unsigned(k * 11, 32));
      wait for 1 ns;
      s := s + rx.n + to_integer(unsigned(low));
      if hit then s := s + 1; end if;
      if en = '1' then s := s + 3; end if;
    end loop;
    report "s=" & integer'image(s);
    wait;
  end process;
end;
