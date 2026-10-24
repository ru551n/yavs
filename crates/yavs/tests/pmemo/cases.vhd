-- Pure functions of vectors, integers and reals called with arguments that repeat, from one
-- process: the memo of pure calls (#701) must change nothing but the work done. Covered: a
-- recursive function, a function that reports (every call must report), the same bits under
-- `to` and `downto` descriptors, slices, reversed ranges, real and integer-array arguments,
-- string and unsigned results, a package constant read in the body, and memoised functions called
-- from a function that is not memoised itself (calls between compiled functions).
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
package ppmemo_cases_pkg is
  constant K : integer := 7;
  type int_vec is array (natural range <>) of integer;
  function fib(n : natural) return natural;
  function chk(a : std_logic_vector; b : integer) return std_logic_vector;
  function rev(a : std_logic_vector) return std_logic_vector;
  function lo(a : std_logic_vector) return integer;
  function sum(a : int_vec; s : real) return real;
  function txt(a : std_logic_vector) return string;
  function rep(n : integer; a : unsigned) return unsigned;
  function pw(a : unsigned; n : natural) return unsigned;
  function bigsum(a : int_vec) return integer;
end package;
package body ppmemo_cases_pkg is
  function fib(n : natural) return natural is
  begin
    if n < 2 then return n; end if;
    return fib(n - 1) + fib(n - 2);
  end function;
  -- reports for b = 3
  function chk(a : std_logic_vector; b : integer) return std_logic_vector is
    variable r : std_logic_vector(a'range) := a;
  begin
    for i in a'range loop
      r(i) := a(i) xor to_unsigned(b + K, 8)(i mod 8);
    end loop;
    assert b /= 3 report "chk: b is three" severity warning;
    return r;
  end function;
  function rev(a : std_logic_vector) return std_logic_vector is
    variable r : std_logic_vector(a'reverse_range);
  begin
    for i in a'range loop r(i) := a(i); end loop;
    return r;
  end function;
  function lo(a : std_logic_vector) return integer is
    variable s : integer := a'left * 100 + a'right;
  begin
    for i in a'range loop
      if a(i) = '1' then s := s + 1; end if;
    end loop;
    return s;
  end function;
  function sum(a : int_vec; s : real) return real is
    variable t : real := s;
  begin
    for i in a'range loop t := t + real(a(i)) * 0.5; end loop;
    return t;
  end function;
  function txt(a : std_logic_vector) return string is
    variable s : string(1 to a'length);
    variable k : natural := 1;
  begin
    for i in a'range loop
      if a(i) = '1' then s(k) := 'x'; else s(k) := '.'; end if;
      k := k + 1;
    end loop;
    return s;
  end function;
  function rep(n : integer; a : unsigned) return unsigned is
    variable r : unsigned(a'range) := a;
  begin
    for i in 1 to n loop r := r + 1; end loop;
    return r;
  end function;
  function pw(a : unsigned; n : natural) return unsigned is
  begin
    if n = 0 then return to_unsigned(1, a'length); end if;
    return resize(pw(a, n - 1) * a, a'length);
  end function;
  -- An argument over the 1 KiB the memo keys: the call is made as usual, every time.
  function bigsum(a : int_vec) return integer is
    variable t : integer := 0;
  begin
    for i in a'range loop t := t + a(i) mod 7; end loop;
    return t;
  end function;
end package body;

library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all; use work.ppmemo_cases_pkg.all;
entity pmemo_cases is end;
architecture a of pmemo_cases is
  signal clk : std_logic := '0';
  signal v : std_logic_vector(7 downto 0) := x"a5";
  signal w : std_logic_vector(0 to 7) := x"3c";
  signal done : boolean := false;
  -- Reads a signal, so it is no candidate itself; its calls of memoised functions are.
  -- (the allocation after a kept call needs the temporary area the call grew)
  impure function peek(k : integer) return integer is
    type str_ptr is access string;
    variable p : str_ptr;
    variable t : integer := 0;
  begin
    for i in 0 to k loop
      t := t + lo(v) + lo(rev(w)) + to_integer(unsigned(chk(v, i)));
      p := new string'(txt(v) & txt(w));
      t := t + p'length;
      deallocate(p);
    end loop;
    return t;
  end function;
begin
  clk <= not clk after 5 ns when not done else clk;
  process
    variable acc : integer := 0;
    variable x : std_logic_vector(7 downto 0);
    variable y : std_logic_vector(0 to 7);
    variable iv : int_vec(0 to 3) := (1, 2, 3, 4);
    variable jv : int_vec(5 downto 2) := (1, 2, 3, 4);
    variable r : real := 0.0;
    variable u : unsigned(7 downto 0) := x"05";
    variable big : int_vec(0 to 299) := (others => 3);
  begin
    for c in 1 to 150 loop
      wait until rising_edge(clk);
      v <= std_logic_vector(unsigned(v) + 3);
      w <= w(1 to 7) & w(0);
      acc := acc + fib(c mod 12);
      x := chk(v, c mod 5);
      y := chk(w, c mod 5);
      acc := acc + to_integer(unsigned(x)) + to_integer(unsigned(y));
      acc := acc + lo(v) + lo(w) + lo(rev(v)) + lo(rev(w)) + lo(v(3 downto 0)) + lo(v(7 downto 4)) + lo(w(0 to 3)) + lo(w(4 to 7));
      r := r + sum(iv, real(c mod 3)) + sum(jv, real(c mod 3));
      if c mod 10 = 0 then report txt(v) & " " & txt(w(2 to 5)); end if;
      u := rep(c mod 4, u);
      acc := acc + to_integer(pw(u, c mod 5));
      acc := acc + peek(c mod 4);
      big(c mod 300) := c;
      acc := acc + bigsum(big);
    end loop;
    report "acc=" & integer'image(acc) & " r=" & real'image(r);
    done <= true;
    wait;
  end process;
end;
