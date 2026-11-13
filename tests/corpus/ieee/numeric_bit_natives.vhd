-- difftest: top=numeric_bit_natives std=2008
-- numeric_bit unary "-" and "abs" of signed and minimum and maximum of two unsigned or two signed
-- vectors, against element loops and the numeric_bit "<": the most negative value, zero, null
-- arrays (a null result), different lengths (the shorter one extended), widths
-- around the 64-element blocks.
library ieee;
use ieee.numeric_bit.all;
entity numeric_bit_natives is end entity;
architecture a of numeric_bit_natives is
  type widths_t is array (natural range <>) of natural;
  constant widths : widths_t := (1, 2, 8, 63, 64, 65, 130);
begin
  process
    variable acc, cases, bad : natural := 0;
    variable seed : natural := 991;

    impure function rnd(n : positive) return natural is
    begin
      seed := (seed * 75 + 74) mod 65537;
      return seed mod n;
    end function;

    procedure ok(c : boolean; what : string) is
    begin
      cases := cases + 1;
      if not c then
        bad := bad + 1;
        report "mismatch: " & what severity error;
      end if;
    end procedure;

    procedure mix(x : bit_vector) is
    begin
      for i in x'range loop acc := (acc * 31 + bit'pos(x(i))) mod 1000003; end loop;
      acc := (acc * 31 + x'length) mod 1000003;
    end procedure;

    -- Two's complement negation by a carry loop.
    function ng(x : signed) return signed is
      alias v : signed(x'length - 1 downto 0) is x;
      variable r : signed(x'length - 1 downto 0);
      variable c : bit := '1';
    begin
      for i in 0 to x'length - 1 loop r(i) := not v(i) xor c; c := c and not v(i); end loop;
      return r;
    end function;

    function len(x : bit_vector) return natural is
    begin
      return x'length;
    end function;

    procedure one(n : natural; kind : natural) is
      variable s, t : signed(n + 3 downto 4);
      variable u : unsigned(n - 1 downto 0);
      variable w : unsigned(n + 1 downto 0);
      variable mx, mn : unsigned(n + 1 downto 0);
    begin
      for i in s'range loop s(i) := bit'val(rnd(2)); t(i) := bit'val(rnd(2)); end loop;
      if kind = 1 then s := (others => '0'); s(s'left) := '1'; end if;
      if kind = 2 then s := (others => '0'); end if;
      for i in u'range loop u(i) := bit'val(rnd(2)); end loop;
      for i in w'range loop w(i) := bit'val(rnd(2)); end loop;
      ok(-s = ng(s), "neg");
      if s(s'left) = '1' then
        ok(abs s = ng(s), "abs negative");
      else
        ok(abs s = s, "abs");
      end if;
      if s < t then
        ok(minimum(s, t) = s and maximum(s, t) = t, "signed min/max");
      else
        ok(minimum(s, t) = t and maximum(s, t) = s, "signed min/max");
      end if;
      -- Different lengths: u is extended to the length of w.
      if resize(u, n + 2) < w then mx := w; else mx := resize(u, n + 2); end if;
      ok(maximum(u, w) = mx, "unsigned max");
      if mx = w then mn := resize(u, n + 2); else mn := w; end if;
      ok(minimum(w, u) = mn, "unsigned min");
      mix(bit_vector(-s));
      mix(bit_vector(abs s));
      mix(bit_vector(minimum(u, w)));
    end procedure;

    variable z : signed(0 downto 1);
    variable zu : unsigned(0 downto 1);
    variable q : unsigned(3 downto 0) := "1010";
  begin
    for wi in widths'range loop
      for kind in 0 to 2 loop
        one(widths(wi), kind);
      end loop;
    end loop;
    -- (numeric_bit "=" is FALSE for a null operand: compare the lengths.)
    ok(len(bit_vector(-z)) = 0 and len(bit_vector(abs z)) = 0, "null signed");
    ok(len(bit_vector(minimum(zu, q))) = 0 and len(bit_vector(maximum(q, zu))) = 0, "null unsigned");
    report "numeric_bit_natives cases=" & integer'image(cases) & " acc=" & integer'image(acc)
      & " bad=" & integer'image(bad);
    assert bad = 0 severity failure;
    wait;
  end process;
end architecture;
