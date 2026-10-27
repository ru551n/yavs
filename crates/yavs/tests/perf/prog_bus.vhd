-- Address decode and bus glue of the shapes a CPU design is full of: an expression tree, a
-- compound condition, a chain of three arms and a state test, the inputs changed 40 times:
-- the kernel computes them itself (#788), with no process code.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity prog_bus is end;
architecture a of prog_bus is
  type state_t is (S_IDLE, S_REQ, S_RSP);
  constant EN_C : boolean := true;
  signal st : state_t := S_IDLE;
  signal en, rw, ack : std_logic := '0';
  signal addr, rd0, rd1, rdx : std_logic_vector(31 downto 0) := (others => '0');
  signal sel0, sel1, we, busy, x : std_logic;
  signal rdata : std_logic_vector(31 downto 0);
begin
  sel0 <= '1' when EN_C and (addr(31 downto 8) = x"000010") else '0';
  sel1 <= '1' when en = '1' and (addr(31 downto 8) = x"000020") and st /= S_IDLE else '0';
  we <= (en and rw) and not ack;
  busy <= '1' when st = S_REQ else '0';
  rdata <= rd0 when sel0 = '1' else rd1 when sel1 = '1' else (others => '0');
  rdx <= (rd0 and rd1) or (rdata xor x"0000ffff");
  x <= (sel0 or sel1) and (we xor busy);

  process
    variable s : integer := 0;
  begin
    for k in 1 to 40 loop
      en <= std_logic'val(2 + k mod 2);
      rw <= std_logic'val(2 + k / 2 mod 2);
      ack <= std_logic'val(2 + k / 4 mod 2);
      st <= state_t'val(k mod 3);
      addr <= std_logic_vector(to_unsigned((16 + k mod 3 * 16) * 256 + k, 32));
      rd0 <= std_logic_vector(to_unsigned(k * 11, 32));
      rd1 <= std_logic_vector(to_unsigned(k * 13, 32));
      wait for 1 ns;
      s := s + to_integer(unsigned(rdx(15 downto 0)));
      if x = '1' then s := s + 1; end if;
    end loop;
    report "s=" & integer'image(s);
    wait;
  end process;
end;
