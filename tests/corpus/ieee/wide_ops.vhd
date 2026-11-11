-- difftest: top=wide_ops std=2008
-- numeric_std and std_logic_1164 operations on 65 to 192 elements (sums, differences, products,
-- reductions, is_x, the logical operators) and concatenations of up to 128 elements, with
-- metavalues, L and H now and then: in a function and in a process.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity wide_ops is end;
architecture a of wide_ops is
  function h(v : std_ulogic_vector) return integer is
    variable r : integer := 7;
  begin
    for i in v'range loop
      r := (r * 11 + std_ulogic'pos(v(i))) mod 99991;
    end loop;
    return r;
  end function;
  function hu(v : unsigned) return integer is begin return h(std_ulogic_vector(v)); end function;
  function hs(v : signed) return integer is begin return h(std_ulogic_vector(v)); end function;
  function c(x : std_ulogic) return integer is begin return std_ulogic'pos(x); end function;
  function line(k, n : integer) return string is
  begin
    return integer'image(k) & " " & integer'image(n);
  end function;
  function mix(x, y : std_ulogic_vector(63 downto 0); k : integer) return integer is
    variable a65 : std_ulogic_vector(64 downto 0) := (x(63) and y(0)) & x;
    variable b65 : std_ulogic_vector(64 downto 0) := y(5) & y;
    variable a100 : std_ulogic_vector(99 downto 0) := x(35 downto 0) & y;
    variable a128 : std_ulogic_vector(127 downto 0) := x & y;
    variable b128 : std_ulogic_vector(127 downto 0) := y & x(31 downto 0) & y(63 downto 32);
    variable a33 : signed(32 downto 0) := signed(x(32 downto 0));
    variable b33 : signed(32 downto 0) := signed(y(40 downto 8));
    variable n : integer;
  begin
    n := (h(x and y) + h(x nand y) + h(x nor y) + h(x xnor y) + h(not x)
          + h(x(43 downto 0) and not y(43 downto 0)) + h(a65 and b65) + h(a65 or b65)
          + h(a128 xor b128) + h(a65 nand b65) + h(a128 nor b128) + h(not a128)
          + hu(unsigned(a65) and unsigned(b65)) + hu(not unsigned(a128))
          + hs(signed(a128) xnor signed(b128))) mod 99991;
    n := (n * 3 + c(or a65) + 2 * c(and a65) + 3 * c(xor a65) + 5 * c(nand a128) + 7 * c(nor a128)
          + 11 * c(xnor a100) + 13 * c(and b128(127 downto 63)) + 17 * c(or b128(127 downto 63))
          + 19 * boolean'pos(is_x(a65)) + 23 * boolean'pos(is_x(a128))) mod 99991;
    n := (n * 3 + hu(unsigned(a65) + unsigned(b65)) + hu(unsigned(a128) - unsigned(b128))
          + hs(signed(a65) + signed(b128)) + hs(signed(a100) - signed(a65)) + hu(unsigned(a65) + 1)
          + hu(unsigned('0' & not x) + 1) + hu(k - unsigned(a100) + k) + hs(signed(a128) + (-k))
          + hs(k - signed(a65)) + hu(unsigned(a128) + unsigned(x)) + hs(signed(x) - signed(a65)))
         mod 99991;
    n := (n * 3 + hs(signed(a65) * signed(b65)) + hu(unsigned(a65) * unsigned(b65)) + hs(a33 * b33)
          + hu(unsigned(x) * unsigned(y)) + hs(signed(a128) * signed(x))
          + hu(unsigned(a100) * unsigned(y(27 downto 0)))
          + hs(signed(a128) * signed(b128(127 downto 64))) + hu(unsigned(a128) * unsigned(b128))
          + hu(unsigned(a33) * unsigned(a100(30 downto 0)))) mod 99991;
    n := (n * 3 + h(x(29 downto 0) & y(49 downto 0) & x(39 downto 0)) + h(y(0) & x & y(62 downto 0))
          + h('1' & x & 'U') + h(a65 & y(62 downto 0))
          + h(x(10 downto 0) & a100(99 downto 2) & y(3)) + h(y(63 downto 1) & x(63) & x)) mod 99991;
    return n;
  end function;
begin
  process
    variable seed : unsigned(63 downto 0) := x"9e3779b97f4a7c15";
    variable x, y : std_ulogic_vector(63 downto 0);
    variable a65 : std_ulogic_vector(64 downto 0);
    variable p130 : std_ulogic_vector(129 downto 0);
    variable d : std_ulogic_vector(127 downto 0);
    variable u : unsigned(64 downto 0);
    variable n : integer := 0;
  begin
    for k in 0 to 120 loop
      seed := seed(62 downto 0) & (seed(63) xor seed(62) xor seed(60) xor seed(59));
      x := std_ulogic_vector(seed);
      y := std_ulogic_vector(seed rol 17);
      if k mod 7 = 3 then
        x(k mod 64) := std_ulogic'val(k mod 9);
      end if;
      if k mod 11 = 4 then
        y((k * 5) mod 64) := 'H';
        y((k * 3) mod 64) := 'L';
      end if;
      if k mod 13 = 6 then
        y(63) := std_ulogic'val((k / 13) mod 9);
      end if;
      -- In the process itself.
      a65 := (x(63) and y(9)) & x;
      p130 := std_ulogic_vector(signed(a65) * signed(y(0) & y));
      d := std_ulogic_vector(unsigned(x & y) + unsigned(p130(127 downto 0)));
      u := unsigned('0' & not x) + 1;
      n := (n * 3 + mix(x, y, k) + h(p130) + h(d) + h(std_ulogic_vector(u))
            + c(or d(127 downto 63)) + c(and d(127 downto 63))
            + h(x(20 downto 0) & y & x(63 downto 21)) + h(x and y) + h(not y)) mod 99991;
      -- Identities on rounds without a metavalue.
      if not is_x(x & y) then
        assert (unsigned(x & y) + unsigned(y & x)) - unsigned(y & x) = unsigned(x & y)
          report "sum" severity failure;
        assert unsigned(a65) * unsigned(y(3) & y) = unsigned(y(3) & y) * unsigned(a65)
          report "product" severity failure;
        assert (xor (x & y)) = ((xor x) xor (xor y)) report "parity" severity failure;
        assert signed(p130) = signed(a65) * signed(y(0) & y) report "same product" severity failure;
      end if;
      report line(k, n);
    end loop;
    report line(-1, n);
    wait;
  end process;
end;
