-- pgo: top=text_closure_tb std=2008 stop=1us analyze=6
-- Reproduces name and overload resolution load: a web of packages that use each other with
-- all-uses and selected-name uses, a generic wrapper instantiated over many types, many small
-- entities, and a testbench full of qualified expressions, aggregates, literals and mixed operators.

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

-- Counting checker used by all tests.
package tc_cnt is
  type counter_t is protected
    procedure hit (ok : boolean; id : natural);
    impure function checks return natural;
    impure function fails return natural;
  end protected counter_t;
end package tc_cnt;

package body tc_cnt is
  type counter_t is protected body
    variable n, bad : natural := 0;

    procedure hit (ok : boolean; id : natural) is
    begin
      n := n + 1;
      if not ok then
        bad := bad + 1;
        report "check " & integer'image(id) & " failed" severity error;
      end if;
    end procedure;

    impure function checks return natural is
    begin
      return n;
    end function;

    impure function fails return natural is
    begin
      return bad;
    end function;
  end protected body counter_t;
end package body tc_cnt;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_cnt.all;

package tc_chk is
  shared variable counter : counter_t;
  procedure ck (c : boolean; id : natural);
  procedure ck (c : std_ulogic; id : natural);
  procedure summary (name : string);
end package tc_chk;

package body tc_chk is
  procedure ck (c : boolean; id : natural) is
  begin
    counter.hit(c, id);
  end procedure;

  procedure ck (c : std_ulogic; id : natural) is
  begin
    counter.hit(c = '1', id);
  end procedure;

  procedure summary (name : string) is
  begin
    report name & ": " & integer'image(counter.checks) & " checks, " & integer'image(counter.fails) & " failed";
    assert counter.fails = 0 report name & " has failures" severity failure;
  end procedure;
end package body tc_chk;

-- Types and overloaded operations for the scenario tests.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package tc_misc is
  type ivec is array (natural range <>) of integer;
  type bvec is array (natural range <>) of boolean;
  type bits_t is array (natural range <>) of bit;
  type c1 is (red, green, blue);
  type c2 is (red, yellow, blue, white);
  type c3 is ('r', 'g', 'b');
  type pt is record
    x, y : integer;
  end record;
  type seg is record
    a, b : pt;
    tag : c1;
  end record;
  type seg_vec is array (natural range <>) of seg;
  type grid is array (0 to 1, 0 to 2) of integer;
  type duration_t is range 0 to 1000000000 units
    tick;
    kilo = 1000 tick;
    mega = 1000 kilo;
  end units duration_t;

  constant origin : pt := (0, 0);
  constant unit_seg : seg := (a => (0, 0), b => (1, 1), tag => green);
  function area (w : integer := 1; h : integer := 1; scale : real := 1.0) return real;
  function area (r : real) return real;
  function pick (a, b : integer) return integer;
  function pick (a, b : real) return real;
  function pick (a : integer; b : real) return real;
  function pick (a : real; b : integer) return real;
  function "+" (a : c1; b : integer) return c1;
  function "+" (a : c2; b : integer) return c2;
  function "-" (a : pt) return pt;
  function "+" (a, b : pt) return pt;
  function "<" (a, b : pt) return boolean;
  function "&" (a, b : pt) return ivec;
  function "+" (a : duration_t; b : integer) return duration_t;
  function "+" (a : ivec; b : integer) return ivec;
  function "*" (a : integer; b : pt) return pt;
  function "*" (a : real; b : pt) return pt;
  procedure pr (x : in integer; y : out integer);
  procedure pr (x : in integer; y : out boolean);
  procedure pr (x : in c1; y : out integer);
  procedure pr (x : in ivec; y : out integer);
  function first_of (v : ivec) return integer;
  function first_of (v : bvec) return boolean;
  function first_of (s : string) return character;
  function first_of (v : std_logic_vector) return std_ulogic;
  function first_of (v : unsigned) return std_ulogic;
  function first_of (v : bit_vector) return bit;
  function len_of (v : ivec) return natural;
  function len_of (s : string) return natural;
  function len_of (v : seg_vec) return natural;
  function name_of (x : c1) return string;
  function name_of (x : c2) return string;
  function name_of (x : c3) return string;
  function name_of (x : bit) return string;
  function name_of (x : std_ulogic) return string;
  function name_of (x : character) return string;
  function name_of (x : boolean) return string;
  function name_of (x : integer) return string;
  function name_of (x : real) return string;
  function name_of (x : time) return string;
  function nest (x : integer) return integer;
  function nest (x : integer; y : integer) return integer;
  function twice (x : integer) return integer;
  function twice (x : real) return real;
  function twice (x : ivec) return ivec;
  alias plus_pt is "+" [pt, pt return pt];
  alias name_c1 is name_of [c1 return string];
  alias blue1 is blue [return c1];
  alias blue2 is blue [return c2];
end package tc_misc;

package body tc_misc is
  function area (w : integer := 1; h : integer := 1; scale : real := 1.0) return real is
  begin
    return real(w * h) * scale;
  end function;

  function area (r : real) return real is
  begin
    return r * r * 3.0;
  end function;

  function pick (a, b : integer) return integer is
  begin
    return a + b;
  end function;

  function pick (a, b : real) return real is
  begin
    return a * b;
  end function;

  function pick (a : integer; b : real) return real is
  begin
    return real(a) - b;
  end function;

  function pick (a : real; b : integer) return real is
  begin
    return a - real(b);
  end function;

  function "+" (a : c1; b : integer) return c1 is
  begin
    return c1'val((c1'pos(a) + b) mod 3);
  end function;

  function "+" (a : c2; b : integer) return c2 is
  begin
    return c2'val((c2'pos(a) + b) mod 4);
  end function;

  function "-" (a : pt) return pt is
  begin
    return (-a.x, -a.y);
  end function;

  function "+" (a, b : pt) return pt is
  begin
    return (a.x + b.x, a.y + b.y);
  end function;

  function "<" (a, b : pt) return boolean is
  begin
    return a.x < b.x or (a.x = b.x and a.y < b.y);
  end function;

  function "&" (a, b : pt) return ivec is
  begin
    return (a.x, a.y, b.x, b.y);
  end function;

  function "+" (a : duration_t; b : integer) return duration_t is
  begin
    return a + b * 1 tick;
  end function;

  function "+" (a : ivec; b : integer) return ivec is
    variable r : ivec(a'range);
  begin
    for i in a'range loop
      r(i) := a(i) + b;
    end loop;
    return r;
  end function;

  function "*" (a : integer; b : pt) return pt is
  begin
    return (a * b.x, a * b.y);
  end function;

  function "*" (a : real; b : pt) return pt is
  begin
    return (integer(a * real(b.x)), integer(a * real(b.y)));
  end function;

  procedure pr (x : in integer; y : out integer) is
  begin
    y := x * 2;
  end procedure;

  procedure pr (x : in integer; y : out boolean) is
  begin
    y := x > 0;
  end procedure;

  procedure pr (x : in c1; y : out integer) is
  begin
    y := c1'pos(x);
  end procedure;

  procedure pr (x : in ivec; y : out integer) is
  begin
    y := x'length;
  end procedure;

  function first_of (v : ivec) return integer is
  begin
    return v(v'left);
  end function;

  function first_of (v : bvec) return boolean is
  begin
    return v(v'left);
  end function;

  function first_of (s : string) return character is
  begin
    return s(s'left);
  end function;

  function first_of (v : std_logic_vector) return std_ulogic is
  begin
    return v(v'left);
  end function;

  function first_of (v : unsigned) return std_ulogic is
  begin
    return v(v'left);
  end function;

  function first_of (v : bit_vector) return bit is
  begin
    return v(v'left);
  end function;

  function len_of (v : ivec) return natural is
  begin
    return v'length;
  end function;

  function len_of (s : string) return natural is
  begin
    return s'length + 100;
  end function;

  function len_of (v : seg_vec) return natural is
  begin
    return v'length + 200;
  end function;

  function name_of (x : c1) return string is
  begin
    return "c1:" & c1'image(x);
  end function;

  function name_of (x : c2) return string is
  begin
    return "c2:" & c2'image(x);
  end function;

  function name_of (x : c3) return string is
  begin
    return "c3:" & c3'image(x);
  end function;

  function name_of (x : bit) return string is
  begin
    return "bit";
  end function;

  function name_of (x : std_ulogic) return string is
  begin
    return "sl";
  end function;

  function name_of (x : character) return string is
  begin
    return "char";
  end function;

  function name_of (x : boolean) return string is
  begin
    return "bool";
  end function;

  function name_of (x : integer) return string is
  begin
    return "int";
  end function;

  function name_of (x : real) return string is
  begin
    return "real";
  end function;

  function name_of (x : time) return string is
  begin
    return "time";
  end function;

  function nest (x : integer) return integer is
    function nest (y : integer) return integer is
    begin
      return y + 1;
    end function;
  begin
    return nest(x) * 10;
  end function;

  function nest (x : integer; y : integer) return integer is
  begin
    return x * 100 + y;
  end function;

  function twice (x : integer) return integer is
  begin
    return 2 * x;
  end function;

  function twice (x : real) return real is
  begin
    return 2.0 * x;
  end function;

  function twice (x : ivec) return ivec is
  begin
    return x + 0 & x;
  end function;
end package body tc_misc;

-- Generic wrapper over any family type.
package tc_wrap is
  generic (
    type t;
    function idx (x : t) return integer;
    function from_idx (i : integer) return t
  );
  function twice (x : t) return t;
  function same (a, b : t) return boolean;
  function shifted (x : t; k : integer) return t;
end package tc_wrap;

package body tc_wrap is
  function twice (x : t) return t is
  begin
    return from_idx(2 * idx(x));
  end function;

  function same (a, b : t) return boolean is
  begin
    return idx(a) = idx(b);
  end function;

  function shifted (x : t; k : integer) return t is
  begin
    return from_idx(idx(x) + k);
  end function;
end package body tc_wrap;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

-- Package 00: family 0 type with overloaded accessors.
package tc_p00 is
  type t00 is range 0 to 1000;
  constant z00 : t00 := 0;
  function idx (x : t00) return integer;
  function from_idx (i : integer) return t00;
  function f (x : t00) return t00;
  function f (x : t00; k : integer) return integer;
  function f (s : string) return t00;
  function conv return t00;
  procedure bump (x : inout t00);
  procedure parts (x : in t00; a : out integer; b : out boolean);
  alias f00 is f [t00 return t00];
end package tc_p00;

package body tc_p00 is
  function idx (x : t00) return integer is
  begin
    return integer(x);
  end function;

  function from_idx (i : integer) return t00 is
  begin
    return t00(i mod 1001);
  end function;

  function f (x : t00) return t00 is
  begin
    return t00((integer(x) * 3 + 0) mod 1001);
  end function;

  function f (x : t00; k : integer) return integer is
  begin
    return idx(x) + k;
  end function;

  function f (s : string) return t00 is
  begin
    return t00((s'length + 0) mod 1001);
  end function;

  function conv return t00 is
  begin
    return from_idx(0);
  end function;

  procedure bump (x : inout t00) is
  begin
    x := f(x);
  end procedure;

  procedure parts (x : in t00; a : out integer; b : out boolean) is
  begin
    a := idx(x);
    b := idx(x) mod 2 = 0;
  end procedure;

end package body tc_p00;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.tc_p00.all;

-- Package 01: family 1 type with overloaded accessors.
package tc_p01 is
  type t01 is (idle, busy, stall, w01);
  constant z01 : t01 := idle;
  function idx (x : t01) return integer;
  function from_idx (i : integer) return t01;
  function f (x : t01) return t01;
  function f (x : t01; k : integer) return integer;
  function f (s : string) return t01;
  function conv return t01;
  procedure bump (x : inout t01);
  procedure parts (x : in t01; a : out integer; b : out boolean);
  function g (x : t01; y : t00) return integer;
  alias f01 is f [t01 return t01];
end package tc_p01;

package body tc_p01 is
  function idx (x : t01) return integer is
  begin
    return t01'pos(x);
  end function;

  function from_idx (i : integer) return t01 is
  begin
    return t01'val(i mod 4);
  end function;

  function f (x : t01) return t01 is
  begin
    return t01'val((t01'pos(x) + 2) mod 4);
  end function;

  function f (x : t01; k : integer) return integer is
  begin
    return idx(x) + k;
  end function;

  function f (s : string) return t01 is
  begin
    return t01'val((s'length + 1) mod 4);
  end function;

  function conv return t01 is
  begin
    return t01'val(1);
  end function;

  procedure bump (x : inout t01) is
  begin
    x := f(x);
  end procedure;

  procedure parts (x : in t01; a : out integer; b : out boolean) is
  begin
    a := idx(x);
    b := idx(x) mod 2 = 0;
  end procedure;

  function g (x : t01; y : t00) return integer is
  begin
    return idx(x) * 1000 + idx(y);
  end function;

end package body tc_p01;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.tc_p01.all;
use work.tc_p00.t00;

-- Package 02: family 2 type with overloaded accessors.
package tc_p02 is
  type t02 is array (0 to 3) of std_logic_vector(7 downto 0);
  constant z02 : t02 := (others => (others => '0'));
  function idx (x : t02) return integer;
  function from_idx (i : integer) return t02;
  function f (x : t02) return t02;
  function f (x : t02; k : integer) return integer;
  function f (s : string) return t02;
  function conv return t02;
  procedure bump (x : inout t02);
  procedure parts (x : in t02; a : out integer; b : out boolean);
  function g (x : t02; y : t01) return integer;
  alias f02 is f [t02 return t02];
end package tc_p02;

package body tc_p02 is
  function idx (x : t02) return integer is
  begin
    return to_integer(unsigned(x(0)));
  end function;

  function from_idx (i : integer) return t02 is
    variable r : t02;
  begin
    for k in 0 to 3 loop
      r(k) := std_logic_vector(to_unsigned((i * (k + 1)) mod 256, 8));
    end loop;
    return r;
  end function;

  function f (x : t02) return t02 is
  begin
    return (x(1), x(2), x(3), x(0));
  end function;

  function f (x : t02; k : integer) return integer is
  begin
    return idx(x) + k;
  end function;

  function f (s : string) return t02 is
  begin
    return from_idx(s'length + 2);
  end function;

  function conv return t02 is
  begin
    return from_idx(2);
  end function;

  procedure bump (x : inout t02) is
  begin
    x := f(x);
  end procedure;

  procedure parts (x : in t02; a : out integer; b : out boolean) is
  begin
    a := idx(x);
    b := idx(x) mod 2 = 0;
  end procedure;

  function g (x : t02; y : t01) return integer is
  begin
    return idx(x) * 1000 + idx(y);
  end function;

end package body tc_p02;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.tc_p02.all;
use work.tc_p01.t01;
use work.tc_p00.idx;
use work.tc_p00.from_idx;

-- Package 03: family 3 type with overloaded accessors.
package tc_p03 is
  type t03 is record
    a : integer range 0 to 255;
    b : std_logic_vector(3 downto 0);
    c : boolean;
  end record;
  constant z03 : t03 := (0, "0000", false);
  function idx (x : t03) return integer;
  function from_idx (i : integer) return t03;
  function f (x : t03) return t03;
  function f (x : t03; k : integer) return integer;
  function f (s : string) return t03;
  function conv return t03;
  procedure bump (x : inout t03);
  procedure parts (x : in t03; a : out integer; b : out boolean);
  function g (x : t03; y : t02) return integer;
  function "+" (a, b : t03) return t03;
  alias f03 is f [t03 return t03];
end package tc_p03;

package body tc_p03 is
  function idx (x : t03) return integer is
  begin
    return x.a;
  end function;

  function from_idx (i : integer) return t03 is
  begin
    return (a => i mod 256, b => std_logic_vector(to_unsigned(i mod 16, 4)), c => (i mod 2 = 0));
  end function;

  function f (x : t03) return t03 is
  begin
    return (a => (x.a * 2) mod 256, b => not x.b, c => not x.c);
  end function;

  function f (x : t03; k : integer) return integer is
  begin
    return idx(x) + k;
  end function;

  function f (s : string) return t03 is
  begin
    return from_idx(s'length + 3);
  end function;

  function conv return t03 is
  begin
    return from_idx(3);
  end function;

  procedure bump (x : inout t03) is
  begin
    x := f(x);
  end procedure;

  procedure parts (x : in t03; a : out integer; b : out boolean) is
  begin
    a := idx(x);
    b := idx(x) mod 2 = 0;
  end procedure;

  function g (x : t03; y : t02) return integer is
  begin
    return idx(x) * 1000 + idx(y);
  end function;

  function "+" (a, b : t03) return t03 is
  begin
    return from_idx(a.a + b.a);
  end function;

end package body tc_p03;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.tc_p03.all;
use work.tc_p02.t02;
use work.tc_p01.idx;
use work.tc_p01.from_idx;

-- Package 04: family 0 type with overloaded accessors.
package tc_p04 is
  type t04 is range 0 to 1000;
  constant z04 : t04 := 0;
  function idx (x : t04) return integer;
  function from_idx (i : integer) return t04;
  function f (x : t04) return t04;
  function f (x : t04; k : integer) return integer;
  function f (s : string) return t04;
  function conv return t04;
  procedure bump (x : inout t04);
  procedure parts (x : in t04; a : out integer; b : out boolean);
  function g (x : t04; y : t03) return integer;
  alias f04 is f [t04 return t04];
end package tc_p04;

package body tc_p04 is
  function idx (x : t04) return integer is
  begin
    return integer(x);
  end function;

  function from_idx (i : integer) return t04 is
  begin
    return t04(i mod 1001);
  end function;

  function f (x : t04) return t04 is
  begin
    return t04((integer(x) * 3 + 4) mod 1001);
  end function;

  function f (x : t04; k : integer) return integer is
  begin
    return idx(x) + k;
  end function;

  function f (s : string) return t04 is
  begin
    return t04((s'length + 4) mod 1001);
  end function;

  function conv return t04 is
  begin
    return from_idx(4);
  end function;

  procedure bump (x : inout t04) is
  begin
    x := f(x);
  end procedure;

  procedure parts (x : in t04; a : out integer; b : out boolean) is
  begin
    a := idx(x);
    b := idx(x) mod 2 = 0;
  end procedure;

  function g (x : t04; y : t03) return integer is
  begin
    return idx(x) * 1000 + idx(y);
  end function;

end package body tc_p04;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.tc_p04.all;
use work.tc_p03.t03;
use work.tc_p02.idx;
use work.tc_p02.from_idx;

-- Package 05: family 1 type with overloaded accessors.
package tc_p05 is
  type t05 is (idle, busy, stall, w05);
  constant z05 : t05 := idle;
  function idx (x : t05) return integer;
  function from_idx (i : integer) return t05;
  function f (x : t05) return t05;
  function f (x : t05; k : integer) return integer;
  function f (s : string) return t05;
  function conv return t05;
  procedure bump (x : inout t05);
  procedure parts (x : in t05; a : out integer; b : out boolean);
  function g (x : t05; y : t04) return integer;
  alias f05 is f [t05 return t05];
end package tc_p05;

package body tc_p05 is
  function idx (x : t05) return integer is
  begin
    return t05'pos(x);
  end function;

  function from_idx (i : integer) return t05 is
  begin
    return t05'val(i mod 4);
  end function;

  function f (x : t05) return t05 is
  begin
    return t05'val((t05'pos(x) + 6) mod 4);
  end function;

  function f (x : t05; k : integer) return integer is
  begin
    return idx(x) + k;
  end function;

  function f (s : string) return t05 is
  begin
    return t05'val((s'length + 5) mod 4);
  end function;

  function conv return t05 is
  begin
    return t05'val(1);
  end function;

  procedure bump (x : inout t05) is
  begin
    x := f(x);
  end procedure;

  procedure parts (x : in t05; a : out integer; b : out boolean) is
  begin
    a := idx(x);
    b := idx(x) mod 2 = 0;
  end procedure;

  function g (x : t05; y : t04) return integer is
  begin
    return idx(x) * 1000 + idx(y);
  end function;

end package body tc_p05;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.tc_p05.all;
use work.tc_p04.t04;
use work.tc_p03.idx;
use work.tc_p03.from_idx;

-- Package 06: family 2 type with overloaded accessors.
package tc_p06 is
  type t06 is array (0 to 3) of std_logic_vector(7 downto 0);
  constant z06 : t06 := (others => (others => '0'));
  function idx (x : t06) return integer;
  function from_idx (i : integer) return t06;
  function f (x : t06) return t06;
  function f (x : t06; k : integer) return integer;
  function f (s : string) return t06;
  function conv return t06;
  procedure bump (x : inout t06);
  procedure parts (x : in t06; a : out integer; b : out boolean);
  function g (x : t06; y : t05) return integer;
  alias f06 is f [t06 return t06];
end package tc_p06;

package body tc_p06 is
  function idx (x : t06) return integer is
  begin
    return to_integer(unsigned(x(0)));
  end function;

  function from_idx (i : integer) return t06 is
    variable r : t06;
  begin
    for k in 0 to 3 loop
      r(k) := std_logic_vector(to_unsigned((i * (k + 1)) mod 256, 8));
    end loop;
    return r;
  end function;

  function f (x : t06) return t06 is
  begin
    return (x(1), x(2), x(3), x(0));
  end function;

  function f (x : t06; k : integer) return integer is
  begin
    return idx(x) + k;
  end function;

  function f (s : string) return t06 is
  begin
    return from_idx(s'length + 6);
  end function;

  function conv return t06 is
  begin
    return from_idx(6);
  end function;

  procedure bump (x : inout t06) is
  begin
    x := f(x);
  end procedure;

  procedure parts (x : in t06; a : out integer; b : out boolean) is
  begin
    a := idx(x);
    b := idx(x) mod 2 = 0;
  end procedure;

  function g (x : t06; y : t05) return integer is
  begin
    return idx(x) * 1000 + idx(y);
  end function;

end package body tc_p06;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.tc_p06.all;
use work.tc_p05.t05;
use work.tc_p04.idx;
use work.tc_p04.from_idx;

-- Package 07: family 3 type with overloaded accessors.
package tc_p07 is
  type t07 is record
    a : integer range 0 to 255;
    b : std_logic_vector(3 downto 0);
    c : boolean;
  end record;
  constant z07 : t07 := (0, "0000", false);
  function idx (x : t07) return integer;
  function from_idx (i : integer) return t07;
  function f (x : t07) return t07;
  function f (x : t07; k : integer) return integer;
  function f (s : string) return t07;
  function conv return t07;
  procedure bump (x : inout t07);
  procedure parts (x : in t07; a : out integer; b : out boolean);
  function g (x : t07; y : t06) return integer;
  function "+" (a, b : t07) return t07;
  alias f07 is f [t07 return t07];
end package tc_p07;

package body tc_p07 is
  function idx (x : t07) return integer is
  begin
    return x.a;
  end function;

  function from_idx (i : integer) return t07 is
  begin
    return (a => i mod 256, b => std_logic_vector(to_unsigned(i mod 16, 4)), c => (i mod 2 = 0));
  end function;

  function f (x : t07) return t07 is
  begin
    return (a => (x.a * 2) mod 256, b => not x.b, c => not x.c);
  end function;

  function f (x : t07; k : integer) return integer is
  begin
    return idx(x) + k;
  end function;

  function f (s : string) return t07 is
  begin
    return from_idx(s'length + 7);
  end function;

  function conv return t07 is
  begin
    return from_idx(7);
  end function;

  procedure bump (x : inout t07) is
  begin
    x := f(x);
  end procedure;

  procedure parts (x : in t07; a : out integer; b : out boolean) is
  begin
    a := idx(x);
    b := idx(x) mod 2 = 0;
  end procedure;

  function g (x : t07; y : t06) return integer is
  begin
    return idx(x) * 1000 + idx(y);
  end function;

  function "+" (a, b : t07) return t07 is
  begin
    return from_idx(a.a + b.a);
  end function;

end package body tc_p07;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.tc_p07.all;
use work.tc_p06.t06;
use work.tc_p05.idx;
use work.tc_p05.from_idx;

-- Package 08: family 0 type with overloaded accessors.
package tc_p08 is
  type t08 is range 0 to 1000;
  constant z08 : t08 := 0;
  function idx (x : t08) return integer;
  function from_idx (i : integer) return t08;
  function f (x : t08) return t08;
  function f (x : t08; k : integer) return integer;
  function f (s : string) return t08;
  function conv return t08;
  procedure bump (x : inout t08);
  procedure parts (x : in t08; a : out integer; b : out boolean);
  function g (x : t08; y : t07) return integer;
  alias f08 is f [t08 return t08];
end package tc_p08;

package body tc_p08 is
  function idx (x : t08) return integer is
  begin
    return integer(x);
  end function;

  function from_idx (i : integer) return t08 is
  begin
    return t08(i mod 1001);
  end function;

  function f (x : t08) return t08 is
  begin
    return t08((integer(x) * 3 + 8) mod 1001);
  end function;

  function f (x : t08; k : integer) return integer is
  begin
    return idx(x) + k;
  end function;

  function f (s : string) return t08 is
  begin
    return t08((s'length + 8) mod 1001);
  end function;

  function conv return t08 is
  begin
    return from_idx(8);
  end function;

  procedure bump (x : inout t08) is
  begin
    x := f(x);
  end procedure;

  procedure parts (x : in t08; a : out integer; b : out boolean) is
  begin
    a := idx(x);
    b := idx(x) mod 2 = 0;
  end procedure;

  function g (x : t08; y : t07) return integer is
  begin
    return idx(x) * 1000 + idx(y);
  end function;

end package body tc_p08;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.tc_p08.all;
use work.tc_p07.t07;
use work.tc_p06.idx;
use work.tc_p06.from_idx;

-- Package 09: family 1 type with overloaded accessors.
package tc_p09 is
  type t09 is (idle, busy, stall, w09);
  constant z09 : t09 := idle;
  function idx (x : t09) return integer;
  function from_idx (i : integer) return t09;
  function f (x : t09) return t09;
  function f (x : t09; k : integer) return integer;
  function f (s : string) return t09;
  function conv return t09;
  procedure bump (x : inout t09);
  procedure parts (x : in t09; a : out integer; b : out boolean);
  function g (x : t09; y : t08) return integer;
  alias f09 is f [t09 return t09];
end package tc_p09;

package body tc_p09 is
  function idx (x : t09) return integer is
  begin
    return t09'pos(x);
  end function;

  function from_idx (i : integer) return t09 is
  begin
    return t09'val(i mod 4);
  end function;

  function f (x : t09) return t09 is
  begin
    return t09'val((t09'pos(x) + 10) mod 4);
  end function;

  function f (x : t09; k : integer) return integer is
  begin
    return idx(x) + k;
  end function;

  function f (s : string) return t09 is
  begin
    return t09'val((s'length + 9) mod 4);
  end function;

  function conv return t09 is
  begin
    return t09'val(1);
  end function;

  procedure bump (x : inout t09) is
  begin
    x := f(x);
  end procedure;

  procedure parts (x : in t09; a : out integer; b : out boolean) is
  begin
    a := idx(x);
    b := idx(x) mod 2 = 0;
  end procedure;

  function g (x : t09; y : t08) return integer is
  begin
    return idx(x) * 1000 + idx(y);
  end function;

end package body tc_p09;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.tc_p09.all;
use work.tc_p08.t08;
use work.tc_p07.idx;
use work.tc_p07.from_idx;

-- Package 10: family 2 type with overloaded accessors.
package tc_p10 is
  type t10 is array (0 to 3) of std_logic_vector(7 downto 0);
  constant z10 : t10 := (others => (others => '0'));
  function idx (x : t10) return integer;
  function from_idx (i : integer) return t10;
  function f (x : t10) return t10;
  function f (x : t10; k : integer) return integer;
  function f (s : string) return t10;
  function conv return t10;
  procedure bump (x : inout t10);
  procedure parts (x : in t10; a : out integer; b : out boolean);
  function g (x : t10; y : t09) return integer;
  alias f10 is f [t10 return t10];
end package tc_p10;

package body tc_p10 is
  function idx (x : t10) return integer is
  begin
    return to_integer(unsigned(x(0)));
  end function;

  function from_idx (i : integer) return t10 is
    variable r : t10;
  begin
    for k in 0 to 3 loop
      r(k) := std_logic_vector(to_unsigned((i * (k + 1)) mod 256, 8));
    end loop;
    return r;
  end function;

  function f (x : t10) return t10 is
  begin
    return (x(1), x(2), x(3), x(0));
  end function;

  function f (x : t10; k : integer) return integer is
  begin
    return idx(x) + k;
  end function;

  function f (s : string) return t10 is
  begin
    return from_idx(s'length + 10);
  end function;

  function conv return t10 is
  begin
    return from_idx(10);
  end function;

  procedure bump (x : inout t10) is
  begin
    x := f(x);
  end procedure;

  procedure parts (x : in t10; a : out integer; b : out boolean) is
  begin
    a := idx(x);
    b := idx(x) mod 2 = 0;
  end procedure;

  function g (x : t10; y : t09) return integer is
  begin
    return idx(x) * 1000 + idx(y);
  end function;

end package body tc_p10;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.tc_p10.all;
use work.tc_p09.t09;
use work.tc_p08.idx;
use work.tc_p08.from_idx;

-- Package 11: family 3 type with overloaded accessors.
package tc_p11 is
  type t11 is record
    a : integer range 0 to 255;
    b : std_logic_vector(3 downto 0);
    c : boolean;
  end record;
  constant z11 : t11 := (0, "0000", false);
  function idx (x : t11) return integer;
  function from_idx (i : integer) return t11;
  function f (x : t11) return t11;
  function f (x : t11; k : integer) return integer;
  function f (s : string) return t11;
  function conv return t11;
  procedure bump (x : inout t11);
  procedure parts (x : in t11; a : out integer; b : out boolean);
  function g (x : t11; y : t10) return integer;
  function "+" (a, b : t11) return t11;
  alias f11 is f [t11 return t11];
end package tc_p11;

package body tc_p11 is
  function idx (x : t11) return integer is
  begin
    return x.a;
  end function;

  function from_idx (i : integer) return t11 is
  begin
    return (a => i mod 256, b => std_logic_vector(to_unsigned(i mod 16, 4)), c => (i mod 2 = 0));
  end function;

  function f (x : t11) return t11 is
  begin
    return (a => (x.a * 2) mod 256, b => not x.b, c => not x.c);
  end function;

  function f (x : t11; k : integer) return integer is
  begin
    return idx(x) + k;
  end function;

  function f (s : string) return t11 is
  begin
    return from_idx(s'length + 11);
  end function;

  function conv return t11 is
  begin
    return from_idx(11);
  end function;

  procedure bump (x : inout t11) is
  begin
    x := f(x);
  end procedure;

  procedure parts (x : in t11; a : out integer; b : out boolean) is
  begin
    a := idx(x);
    b := idx(x) mod 2 = 0;
  end procedure;

  function g (x : t11; y : t10) return integer is
  begin
    return idx(x) * 1000 + idx(y);
  end function;

  function "+" (a, b : t11) return t11 is
  begin
    return from_idx(a.a + b.a);
  end function;

end package body tc_p11;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.tc_p11.all;
use work.tc_p10.t10;
use work.tc_p09.idx;
use work.tc_p09.from_idx;

-- Package 12: family 0 type with overloaded accessors.
package tc_p12 is
  type t12 is range 0 to 1000;
  constant z12 : t12 := 0;
  function idx (x : t12) return integer;
  function from_idx (i : integer) return t12;
  function f (x : t12) return t12;
  function f (x : t12; k : integer) return integer;
  function f (s : string) return t12;
  function conv return t12;
  procedure bump (x : inout t12);
  procedure parts (x : in t12; a : out integer; b : out boolean);
  function g (x : t12; y : t11) return integer;
  alias f12 is f [t12 return t12];
end package tc_p12;

package body tc_p12 is
  function idx (x : t12) return integer is
  begin
    return integer(x);
  end function;

  function from_idx (i : integer) return t12 is
  begin
    return t12(i mod 1001);
  end function;

  function f (x : t12) return t12 is
  begin
    return t12((integer(x) * 3 + 12) mod 1001);
  end function;

  function f (x : t12; k : integer) return integer is
  begin
    return idx(x) + k;
  end function;

  function f (s : string) return t12 is
  begin
    return t12((s'length + 12) mod 1001);
  end function;

  function conv return t12 is
  begin
    return from_idx(12);
  end function;

  procedure bump (x : inout t12) is
  begin
    x := f(x);
  end procedure;

  procedure parts (x : in t12; a : out integer; b : out boolean) is
  begin
    a := idx(x);
    b := idx(x) mod 2 = 0;
  end procedure;

  function g (x : t12; y : t11) return integer is
  begin
    return idx(x) * 1000 + idx(y);
  end function;

end package body tc_p12;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.tc_p12.all;
use work.tc_p11.t11;
use work.tc_p10.idx;
use work.tc_p10.from_idx;

-- Package 13: family 1 type with overloaded accessors.
package tc_p13 is
  type t13 is (idle, busy, stall, w13);
  constant z13 : t13 := idle;
  function idx (x : t13) return integer;
  function from_idx (i : integer) return t13;
  function f (x : t13) return t13;
  function f (x : t13; k : integer) return integer;
  function f (s : string) return t13;
  function conv return t13;
  procedure bump (x : inout t13);
  procedure parts (x : in t13; a : out integer; b : out boolean);
  function g (x : t13; y : t12) return integer;
  alias f13 is f [t13 return t13];
end package tc_p13;

package body tc_p13 is
  function idx (x : t13) return integer is
  begin
    return t13'pos(x);
  end function;

  function from_idx (i : integer) return t13 is
  begin
    return t13'val(i mod 4);
  end function;

  function f (x : t13) return t13 is
  begin
    return t13'val((t13'pos(x) + 14) mod 4);
  end function;

  function f (x : t13; k : integer) return integer is
  begin
    return idx(x) + k;
  end function;

  function f (s : string) return t13 is
  begin
    return t13'val((s'length + 13) mod 4);
  end function;

  function conv return t13 is
  begin
    return t13'val(1);
  end function;

  procedure bump (x : inout t13) is
  begin
    x := f(x);
  end procedure;

  procedure parts (x : in t13; a : out integer; b : out boolean) is
  begin
    a := idx(x);
    b := idx(x) mod 2 = 0;
  end procedure;

  function g (x : t13; y : t12) return integer is
  begin
    return idx(x) * 1000 + idx(y);
  end function;

end package body tc_p13;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.tc_p13.all;
use work.tc_p12.t12;
use work.tc_p11.idx;
use work.tc_p11.from_idx;

-- Package 14: family 2 type with overloaded accessors.
package tc_p14 is
  type t14 is array (0 to 3) of std_logic_vector(7 downto 0);
  constant z14 : t14 := (others => (others => '0'));
  function idx (x : t14) return integer;
  function from_idx (i : integer) return t14;
  function f (x : t14) return t14;
  function f (x : t14; k : integer) return integer;
  function f (s : string) return t14;
  function conv return t14;
  procedure bump (x : inout t14);
  procedure parts (x : in t14; a : out integer; b : out boolean);
  function g (x : t14; y : t13) return integer;
  alias f14 is f [t14 return t14];
end package tc_p14;

package body tc_p14 is
  function idx (x : t14) return integer is
  begin
    return to_integer(unsigned(x(0)));
  end function;

  function from_idx (i : integer) return t14 is
    variable r : t14;
  begin
    for k in 0 to 3 loop
      r(k) := std_logic_vector(to_unsigned((i * (k + 1)) mod 256, 8));
    end loop;
    return r;
  end function;

  function f (x : t14) return t14 is
  begin
    return (x(1), x(2), x(3), x(0));
  end function;

  function f (x : t14; k : integer) return integer is
  begin
    return idx(x) + k;
  end function;

  function f (s : string) return t14 is
  begin
    return from_idx(s'length + 14);
  end function;

  function conv return t14 is
  begin
    return from_idx(14);
  end function;

  procedure bump (x : inout t14) is
  begin
    x := f(x);
  end procedure;

  procedure parts (x : in t14; a : out integer; b : out boolean) is
  begin
    a := idx(x);
    b := idx(x) mod 2 = 0;
  end procedure;

  function g (x : t14; y : t13) return integer is
  begin
    return idx(x) * 1000 + idx(y);
  end function;

end package body tc_p14;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.tc_p14.all;
use work.tc_p13.t13;
use work.tc_p12.idx;
use work.tc_p12.from_idx;

-- Package 15: family 3 type with overloaded accessors.
package tc_p15 is
  type t15 is record
    a : integer range 0 to 255;
    b : std_logic_vector(3 downto 0);
    c : boolean;
  end record;
  constant z15 : t15 := (0, "0000", false);
  function idx (x : t15) return integer;
  function from_idx (i : integer) return t15;
  function f (x : t15) return t15;
  function f (x : t15; k : integer) return integer;
  function f (s : string) return t15;
  function conv return t15;
  procedure bump (x : inout t15);
  procedure parts (x : in t15; a : out integer; b : out boolean);
  function g (x : t15; y : t14) return integer;
  function "+" (a, b : t15) return t15;
  alias f15 is f [t15 return t15];
end package tc_p15;

package body tc_p15 is
  function idx (x : t15) return integer is
  begin
    return x.a;
  end function;

  function from_idx (i : integer) return t15 is
  begin
    return (a => i mod 256, b => std_logic_vector(to_unsigned(i mod 16, 4)), c => (i mod 2 = 0));
  end function;

  function f (x : t15) return t15 is
  begin
    return (a => (x.a * 2) mod 256, b => not x.b, c => not x.c);
  end function;

  function f (x : t15; k : integer) return integer is
  begin
    return idx(x) + k;
  end function;

  function f (s : string) return t15 is
  begin
    return from_idx(s'length + 15);
  end function;

  function conv return t15 is
  begin
    return from_idx(15);
  end function;

  procedure bump (x : inout t15) is
  begin
    x := f(x);
  end procedure;

  procedure parts (x : in t15; a : out integer; b : out boolean) is
  begin
    a := idx(x);
    b := idx(x) mod 2 = 0;
  end procedure;

  function g (x : t15; y : t14) return integer is
  begin
    return idx(x) * 1000 + idx(y);
  end function;

  function "+" (a, b : t15) return t15 is
  begin
    return from_idx(a.a + b.a);
  end function;

end package body tc_p15;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.tc_p15.all;
use work.tc_p14.t14;
use work.tc_p13.idx;
use work.tc_p13.from_idx;

-- Package 16: family 0 type with overloaded accessors.
package tc_p16 is
  type t16 is range 0 to 1000;
  constant z16 : t16 := 0;
  function idx (x : t16) return integer;
  function from_idx (i : integer) return t16;
  function f (x : t16) return t16;
  function f (x : t16; k : integer) return integer;
  function f (s : string) return t16;
  function conv return t16;
  procedure bump (x : inout t16);
  procedure parts (x : in t16; a : out integer; b : out boolean);
  function g (x : t16; y : t15) return integer;
  alias f16 is f [t16 return t16];
end package tc_p16;

package body tc_p16 is
  function idx (x : t16) return integer is
  begin
    return integer(x);
  end function;

  function from_idx (i : integer) return t16 is
  begin
    return t16(i mod 1001);
  end function;

  function f (x : t16) return t16 is
  begin
    return t16((integer(x) * 3 + 16) mod 1001);
  end function;

  function f (x : t16; k : integer) return integer is
  begin
    return idx(x) + k;
  end function;

  function f (s : string) return t16 is
  begin
    return t16((s'length + 16) mod 1001);
  end function;

  function conv return t16 is
  begin
    return from_idx(16);
  end function;

  procedure bump (x : inout t16) is
  begin
    x := f(x);
  end procedure;

  procedure parts (x : in t16; a : out integer; b : out boolean) is
  begin
    a := idx(x);
    b := idx(x) mod 2 = 0;
  end procedure;

  function g (x : t16; y : t15) return integer is
  begin
    return idx(x) * 1000 + idx(y);
  end function;

end package body tc_p16;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.tc_p16.all;
use work.tc_p15.t15;
use work.tc_p14.idx;
use work.tc_p14.from_idx;

-- Package 17: family 1 type with overloaded accessors.
package tc_p17 is
  type t17 is (idle, busy, stall, w17);
  constant z17 : t17 := idle;
  function idx (x : t17) return integer;
  function from_idx (i : integer) return t17;
  function f (x : t17) return t17;
  function f (x : t17; k : integer) return integer;
  function f (s : string) return t17;
  function conv return t17;
  procedure bump (x : inout t17);
  procedure parts (x : in t17; a : out integer; b : out boolean);
  function g (x : t17; y : t16) return integer;
  alias f17 is f [t17 return t17];
end package tc_p17;

package body tc_p17 is
  function idx (x : t17) return integer is
  begin
    return t17'pos(x);
  end function;

  function from_idx (i : integer) return t17 is
  begin
    return t17'val(i mod 4);
  end function;

  function f (x : t17) return t17 is
  begin
    return t17'val((t17'pos(x) + 18) mod 4);
  end function;

  function f (x : t17; k : integer) return integer is
  begin
    return idx(x) + k;
  end function;

  function f (s : string) return t17 is
  begin
    return t17'val((s'length + 17) mod 4);
  end function;

  function conv return t17 is
  begin
    return t17'val(1);
  end function;

  procedure bump (x : inout t17) is
  begin
    x := f(x);
  end procedure;

  procedure parts (x : in t17; a : out integer; b : out boolean) is
  begin
    a := idx(x);
    b := idx(x) mod 2 = 0;
  end procedure;

  function g (x : t17; y : t16) return integer is
  begin
    return idx(x) * 1000 + idx(y);
  end function;

end package body tc_p17;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.tc_p17.all;
use work.tc_p16.t16;
use work.tc_p15.idx;
use work.tc_p15.from_idx;

-- Package 18: family 2 type with overloaded accessors.
package tc_p18 is
  type t18 is array (0 to 3) of std_logic_vector(7 downto 0);
  constant z18 : t18 := (others => (others => '0'));
  function idx (x : t18) return integer;
  function from_idx (i : integer) return t18;
  function f (x : t18) return t18;
  function f (x : t18; k : integer) return integer;
  function f (s : string) return t18;
  function conv return t18;
  procedure bump (x : inout t18);
  procedure parts (x : in t18; a : out integer; b : out boolean);
  function g (x : t18; y : t17) return integer;
  alias f18 is f [t18 return t18];
end package tc_p18;

package body tc_p18 is
  function idx (x : t18) return integer is
  begin
    return to_integer(unsigned(x(0)));
  end function;

  function from_idx (i : integer) return t18 is
    variable r : t18;
  begin
    for k in 0 to 3 loop
      r(k) := std_logic_vector(to_unsigned((i * (k + 1)) mod 256, 8));
    end loop;
    return r;
  end function;

  function f (x : t18) return t18 is
  begin
    return (x(1), x(2), x(3), x(0));
  end function;

  function f (x : t18; k : integer) return integer is
  begin
    return idx(x) + k;
  end function;

  function f (s : string) return t18 is
  begin
    return from_idx(s'length + 18);
  end function;

  function conv return t18 is
  begin
    return from_idx(18);
  end function;

  procedure bump (x : inout t18) is
  begin
    x := f(x);
  end procedure;

  procedure parts (x : in t18; a : out integer; b : out boolean) is
  begin
    a := idx(x);
    b := idx(x) mod 2 = 0;
  end procedure;

  function g (x : t18; y : t17) return integer is
  begin
    return idx(x) * 1000 + idx(y);
  end function;

end package body tc_p18;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.tc_p18.all;
use work.tc_p17.t17;
use work.tc_p16.idx;
use work.tc_p16.from_idx;

-- Package 19: family 3 type with overloaded accessors.
package tc_p19 is
  type t19 is record
    a : integer range 0 to 255;
    b : std_logic_vector(3 downto 0);
    c : boolean;
  end record;
  constant z19 : t19 := (0, "0000", false);
  function idx (x : t19) return integer;
  function from_idx (i : integer) return t19;
  function f (x : t19) return t19;
  function f (x : t19; k : integer) return integer;
  function f (s : string) return t19;
  function conv return t19;
  procedure bump (x : inout t19);
  procedure parts (x : in t19; a : out integer; b : out boolean);
  function g (x : t19; y : t18) return integer;
  function "+" (a, b : t19) return t19;
  alias f19 is f [t19 return t19];
end package tc_p19;

package body tc_p19 is
  function idx (x : t19) return integer is
  begin
    return x.a;
  end function;

  function from_idx (i : integer) return t19 is
  begin
    return (a => i mod 256, b => std_logic_vector(to_unsigned(i mod 16, 4)), c => (i mod 2 = 0));
  end function;

  function f (x : t19) return t19 is
  begin
    return (a => (x.a * 2) mod 256, b => not x.b, c => not x.c);
  end function;

  function f (x : t19; k : integer) return integer is
  begin
    return idx(x) + k;
  end function;

  function f (s : string) return t19 is
  begin
    return from_idx(s'length + 19);
  end function;

  function conv return t19 is
  begin
    return from_idx(19);
  end function;

  procedure bump (x : inout t19) is
  begin
    x := f(x);
  end procedure;

  procedure parts (x : in t19; a : out integer; b : out boolean) is
  begin
    a := idx(x);
    b := idx(x) mod 2 = 0;
  end procedure;

  function g (x : t19; y : t18) return integer is
  begin
    return idx(x) * 1000 + idx(y);
  end function;

  function "+" (a, b : t19) return t19 is
  begin
    return from_idx(a.a + b.a);
  end function;

end package body tc_p19;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p00.all;

package tc_w00 is new work.tc_wrap generic map (t => t00, idx => idx, from_idx => from_idx);

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p01.all;

package tc_w01 is new work.tc_wrap generic map (t => t01, idx => idx, from_idx => from_idx);

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p02.all;

package tc_w02 is new work.tc_wrap generic map (t => t02, idx => idx, from_idx => from_idx);

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p03.all;

package tc_w03 is new work.tc_wrap generic map (t => t03, idx => idx, from_idx => from_idx);

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p04.all;

package tc_w04 is new work.tc_wrap generic map (t => t04, idx => idx, from_idx => from_idx);

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p05.all;

package tc_w05 is new work.tc_wrap generic map (t => t05, idx => idx, from_idx => from_idx);

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p06.all;

package tc_w06 is new work.tc_wrap generic map (t => t06, idx => idx, from_idx => from_idx);

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p07.all;

package tc_w07 is new work.tc_wrap generic map (t => t07, idx => idx, from_idx => from_idx);

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p08.all;

package tc_w08 is new work.tc_wrap generic map (t => t08, idx => idx, from_idx => from_idx);

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p09.all;

package tc_w09 is new work.tc_wrap generic map (t => t09, idx => idx, from_idx => from_idx);

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p10.all;

package tc_w10 is new work.tc_wrap generic map (t => t10, idx => idx, from_idx => from_idx);

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p11.all;

package tc_w11 is new work.tc_wrap generic map (t => t11, idx => idx, from_idx => from_idx);

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p12.all;

package tc_w12 is new work.tc_wrap generic map (t => t12, idx => idx, from_idx => from_idx);

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p13.all;

package tc_w13 is new work.tc_wrap generic map (t => t13, idx => idx, from_idx => from_idx);

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p14.all;

package tc_w14 is new work.tc_wrap generic map (t => t14, idx => idx, from_idx => from_idx);

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p15.all;

package tc_w15 is new work.tc_wrap generic map (t => t15, idx => idx, from_idx => from_idx);

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p16.all;

package tc_w16 is new work.tc_wrap generic map (t => t16, idx => idx, from_idx => from_idx);

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p17.all;

package tc_w17 is new work.tc_wrap generic map (t => t17, idx => idx, from_idx => from_idx);

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p18.all;

package tc_w18 is new work.tc_wrap generic map (t => t18, idx => idx, from_idx => from_idx);

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p19.all;

package tc_w19 is new work.tc_wrap generic map (t => t19, idx => idx, from_idx => from_idx);

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p00.all;

entity tc_e00_reg is
  port (clk : in std_logic; d : in t00; q : out t00 := z00);
end entity tc_e00_reg;

architecture rtl of tc_e00_reg is
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= f(d);
    end if;
  end process;
end architecture rtl;

library ieee;
use ieee.numeric_std.all;
use work.tc_p00.all;

entity tc_e00_cmb is
  port (d : in t00; y : out integer);
end entity tc_e00_cmb;

architecture comb of tc_e00_cmb is
begin
  y <= idx(d) + f(d, 1);
end architecture comb;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p00.all;

entity tc_e00_chain is
  port (clk : in std_logic; d : in t00; q : out t00);
end entity tc_e00_chain;

architecture rtl of tc_e00_chain is
  component tc_e00_reg is
    port (clk : in std_logic; d : in t00; q : out t00 := z00);
  end component;
  for all : tc_e00_reg use entity work.tc_e00_reg(rtl);
  signal mid : t00 := z00;
begin
  u1 : tc_e00_reg port map (clk, d, mid);
  u2 : tc_e00_reg port map (clk => clk, d => mid, q => q);
end architecture rtl;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p01.all;

entity tc_e01_reg is
  port (clk : in std_logic; d : in t01; q : out t01 := z01);
end entity tc_e01_reg;

architecture rtl of tc_e01_reg is
  use work.tc_p06.idx;
  signal dbg : integer := 0;
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= f(d);
      dbg <= idx(d);
    end if;
  end process;
end architecture rtl;

library ieee;
use ieee.numeric_std.all;
use work.tc_p01.all;

entity tc_e01_cmb is
  port (d : in t01; y : out integer);
end entity tc_e01_cmb;

architecture comb of tc_e01_cmb is
begin
  process (all)
    variable a : integer;
    variable b : boolean;
  begin
    parts(d, a, b);
    if b then
      y <= a + f(d, 1);
    else
      y <= f(d, 1) + a;
    end if;
  end process;
end architecture comb;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p01.all;

entity tc_e01_chain is
  port (clk : in std_logic; d : in t01; q : out t01);
end entity tc_e01_chain;

architecture rtl of tc_e01_chain is
  signal mid : t01 := z01;
begin
  u1 : entity work.tc_e01_reg port map (clk, d, mid);
  u2 : entity work.tc_e01_reg(rtl) port map (clk => clk, d => mid, q => q);
end architecture rtl;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p02.all;

entity tc_e02_reg is
  port (clk : in std_logic; d : in t02; q : out t02 := z02);
end entity tc_e02_reg;

architecture rtl of tc_e02_reg is
  alias ff is f [t02 return t02];
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= ff(d);
    end if;
  end process;
end architecture rtl;

library ieee;
use ieee.numeric_std.all;
use work.tc_p02.all;

entity tc_e02_cmb is
  port (d : in t02; y : out integer);
end entity tc_e02_cmb;

architecture comb of tc_e02_cmb is
begin
  y <= idx(d) + f(d, 1);
end architecture comb;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p02.all;

entity tc_e02_chain is
  port (clk : in std_logic; d : in t02; q : out t02);
end entity tc_e02_chain;

architecture rtl of tc_e02_chain is
  component tc_e02_reg is
    port (clk : in std_logic; d : in t02; q : out t02 := z02);
  end component;
  for all : tc_e02_reg use entity work.tc_e02_reg(rtl);
  signal mid : t02 := z02;
begin
  u1 : tc_e02_reg port map (clk, d, mid);
  u2 : tc_e02_reg port map (clk => clk, d => mid, q => q);
end architecture rtl;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p03.all;

entity tc_e03_reg is
  port (clk : in std_logic; d : in t03; q : out t03 := z03);
end entity tc_e03_reg;

architecture rtl of tc_e03_reg is
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= f(d);
    end if;
  end process;
end architecture rtl;

library ieee;
use ieee.numeric_std.all;
use work.tc_p03.all;

entity tc_e03_cmb is
  port (d : in t03; y : out integer);
end entity tc_e03_cmb;

architecture comb of tc_e03_cmb is
begin
  process (all)
    variable a : integer;
    variable b : boolean;
  begin
    parts(d, a, b);
    if b then
      y <= a + f(d, 1);
    else
      y <= f(d, 1) + a;
    end if;
  end process;
end architecture comb;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p03.all;

entity tc_e03_chain is
  port (clk : in std_logic; d : in t03; q : out t03);
end entity tc_e03_chain;

architecture rtl of tc_e03_chain is
  signal mid : t03 := z03;
begin
  u1 : entity work.tc_e03_reg port map (clk, d, mid);
  u2 : entity work.tc_e03_reg(rtl) port map (clk => clk, d => mid, q => q);
end architecture rtl;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p04.all;

entity tc_e04_reg is
  port (clk : in std_logic; d : in t04; q : out t04 := z04);
end entity tc_e04_reg;

architecture rtl of tc_e04_reg is
  use work.tc_p09.idx;
  signal dbg : integer := 0;
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= f(d);
      dbg <= idx(d);
    end if;
  end process;
end architecture rtl;

library ieee;
use ieee.numeric_std.all;
use work.tc_p04.all;

entity tc_e04_cmb is
  port (d : in t04; y : out integer);
end entity tc_e04_cmb;

architecture comb of tc_e04_cmb is
begin
  y <= idx(d) + f(d, 1);
end architecture comb;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p04.all;

entity tc_e04_chain is
  port (clk : in std_logic; d : in t04; q : out t04);
end entity tc_e04_chain;

architecture rtl of tc_e04_chain is
  component tc_e04_reg is
    port (clk : in std_logic; d : in t04; q : out t04 := z04);
  end component;
  for all : tc_e04_reg use entity work.tc_e04_reg(rtl);
  signal mid : t04 := z04;
begin
  u1 : tc_e04_reg port map (clk, d, mid);
  u2 : tc_e04_reg port map (clk => clk, d => mid, q => q);
end architecture rtl;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p05.all;

entity tc_e05_reg is
  port (clk : in std_logic; d : in t05; q : out t05 := z05);
end entity tc_e05_reg;

architecture rtl of tc_e05_reg is
  alias ff is f [t05 return t05];
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= ff(d);
    end if;
  end process;
end architecture rtl;

library ieee;
use ieee.numeric_std.all;
use work.tc_p05.all;

entity tc_e05_cmb is
  port (d : in t05; y : out integer);
end entity tc_e05_cmb;

architecture comb of tc_e05_cmb is
begin
  process (all)
    variable a : integer;
    variable b : boolean;
  begin
    parts(d, a, b);
    if b then
      y <= a + f(d, 1);
    else
      y <= f(d, 1) + a;
    end if;
  end process;
end architecture comb;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p05.all;

entity tc_e05_chain is
  port (clk : in std_logic; d : in t05; q : out t05);
end entity tc_e05_chain;

architecture rtl of tc_e05_chain is
  signal mid : t05 := z05;
begin
  u1 : entity work.tc_e05_reg port map (clk, d, mid);
  u2 : entity work.tc_e05_reg(rtl) port map (clk => clk, d => mid, q => q);
end architecture rtl;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p06.all;

entity tc_e06_reg is
  port (clk : in std_logic; d : in t06; q : out t06 := z06);
end entity tc_e06_reg;

architecture rtl of tc_e06_reg is
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= f(d);
    end if;
  end process;
end architecture rtl;

library ieee;
use ieee.numeric_std.all;
use work.tc_p06.all;

entity tc_e06_cmb is
  port (d : in t06; y : out integer);
end entity tc_e06_cmb;

architecture comb of tc_e06_cmb is
begin
  y <= idx(d) + f(d, 1);
end architecture comb;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p06.all;

entity tc_e06_chain is
  port (clk : in std_logic; d : in t06; q : out t06);
end entity tc_e06_chain;

architecture rtl of tc_e06_chain is
  component tc_e06_reg is
    port (clk : in std_logic; d : in t06; q : out t06 := z06);
  end component;
  for all : tc_e06_reg use entity work.tc_e06_reg(rtl);
  signal mid : t06 := z06;
begin
  u1 : tc_e06_reg port map (clk, d, mid);
  u2 : tc_e06_reg port map (clk => clk, d => mid, q => q);
end architecture rtl;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p07.all;

entity tc_e07_reg is
  port (clk : in std_logic; d : in t07; q : out t07 := z07);
end entity tc_e07_reg;

architecture rtl of tc_e07_reg is
  use work.tc_p12.idx;
  signal dbg : integer := 0;
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= f(d);
      dbg <= idx(d);
    end if;
  end process;
end architecture rtl;

library ieee;
use ieee.numeric_std.all;
use work.tc_p07.all;

entity tc_e07_cmb is
  port (d : in t07; y : out integer);
end entity tc_e07_cmb;

architecture comb of tc_e07_cmb is
begin
  process (all)
    variable a : integer;
    variable b : boolean;
  begin
    parts(d, a, b);
    if b then
      y <= a + f(d, 1);
    else
      y <= f(d, 1) + a;
    end if;
  end process;
end architecture comb;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p07.all;

entity tc_e07_chain is
  port (clk : in std_logic; d : in t07; q : out t07);
end entity tc_e07_chain;

architecture rtl of tc_e07_chain is
  signal mid : t07 := z07;
begin
  u1 : entity work.tc_e07_reg port map (clk, d, mid);
  u2 : entity work.tc_e07_reg(rtl) port map (clk => clk, d => mid, q => q);
end architecture rtl;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p08.all;

entity tc_e08_reg is
  port (clk : in std_logic; d : in t08; q : out t08 := z08);
end entity tc_e08_reg;

architecture rtl of tc_e08_reg is
  alias ff is f [t08 return t08];
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= ff(d);
    end if;
  end process;
end architecture rtl;

library ieee;
use ieee.numeric_std.all;
use work.tc_p08.all;

entity tc_e08_cmb is
  port (d : in t08; y : out integer);
end entity tc_e08_cmb;

architecture comb of tc_e08_cmb is
begin
  y <= idx(d) + f(d, 1);
end architecture comb;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p08.all;

entity tc_e08_chain is
  port (clk : in std_logic; d : in t08; q : out t08);
end entity tc_e08_chain;

architecture rtl of tc_e08_chain is
  component tc_e08_reg is
    port (clk : in std_logic; d : in t08; q : out t08 := z08);
  end component;
  for all : tc_e08_reg use entity work.tc_e08_reg(rtl);
  signal mid : t08 := z08;
begin
  u1 : tc_e08_reg port map (clk, d, mid);
  u2 : tc_e08_reg port map (clk => clk, d => mid, q => q);
end architecture rtl;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p09.all;

entity tc_e09_reg is
  port (clk : in std_logic; d : in t09; q : out t09 := z09);
end entity tc_e09_reg;

architecture rtl of tc_e09_reg is
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= f(d);
    end if;
  end process;
end architecture rtl;

library ieee;
use ieee.numeric_std.all;
use work.tc_p09.all;

entity tc_e09_cmb is
  port (d : in t09; y : out integer);
end entity tc_e09_cmb;

architecture comb of tc_e09_cmb is
begin
  process (all)
    variable a : integer;
    variable b : boolean;
  begin
    parts(d, a, b);
    if b then
      y <= a + f(d, 1);
    else
      y <= f(d, 1) + a;
    end if;
  end process;
end architecture comb;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p09.all;

entity tc_e09_chain is
  port (clk : in std_logic; d : in t09; q : out t09);
end entity tc_e09_chain;

architecture rtl of tc_e09_chain is
  signal mid : t09 := z09;
begin
  u1 : entity work.tc_e09_reg port map (clk, d, mid);
  u2 : entity work.tc_e09_reg(rtl) port map (clk => clk, d => mid, q => q);
end architecture rtl;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p10.all;

entity tc_e10_reg is
  port (clk : in std_logic; d : in t10; q : out t10 := z10);
end entity tc_e10_reg;

architecture rtl of tc_e10_reg is
  use work.tc_p15.idx;
  signal dbg : integer := 0;
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= f(d);
      dbg <= idx(d);
    end if;
  end process;
end architecture rtl;

library ieee;
use ieee.numeric_std.all;
use work.tc_p10.all;

entity tc_e10_cmb is
  port (d : in t10; y : out integer);
end entity tc_e10_cmb;

architecture comb of tc_e10_cmb is
begin
  y <= idx(d) + f(d, 1);
end architecture comb;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p11.all;

entity tc_e11_reg is
  port (clk : in std_logic; d : in t11; q : out t11 := z11);
end entity tc_e11_reg;

architecture rtl of tc_e11_reg is
  alias ff is f [t11 return t11];
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= ff(d);
    end if;
  end process;
end architecture rtl;

library ieee;
use ieee.numeric_std.all;
use work.tc_p11.all;

entity tc_e11_cmb is
  port (d : in t11; y : out integer);
end entity tc_e11_cmb;

architecture comb of tc_e11_cmb is
begin
  process (all)
    variable a : integer;
    variable b : boolean;
  begin
    parts(d, a, b);
    if b then
      y <= a + f(d, 1);
    else
      y <= f(d, 1) + a;
    end if;
  end process;
end architecture comb;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p12.all;

entity tc_e12_reg is
  port (clk : in std_logic; d : in t12; q : out t12 := z12);
end entity tc_e12_reg;

architecture rtl of tc_e12_reg is
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= f(d);
    end if;
  end process;
end architecture rtl;

library ieee;
use ieee.numeric_std.all;
use work.tc_p12.all;

entity tc_e12_cmb is
  port (d : in t12; y : out integer);
end entity tc_e12_cmb;

architecture comb of tc_e12_cmb is
begin
  y <= idx(d) + f(d, 1);
end architecture comb;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p13.all;

entity tc_e13_reg is
  port (clk : in std_logic; d : in t13; q : out t13 := z13);
end entity tc_e13_reg;

architecture rtl of tc_e13_reg is
  use work.tc_p18.idx;
  signal dbg : integer := 0;
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= f(d);
      dbg <= idx(d);
    end if;
  end process;
end architecture rtl;

library ieee;
use ieee.numeric_std.all;
use work.tc_p13.all;

entity tc_e13_cmb is
  port (d : in t13; y : out integer);
end entity tc_e13_cmb;

architecture comb of tc_e13_cmb is
begin
  process (all)
    variable a : integer;
    variable b : boolean;
  begin
    parts(d, a, b);
    if b then
      y <= a + f(d, 1);
    else
      y <= f(d, 1) + a;
    end if;
  end process;
end architecture comb;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p14.all;

entity tc_e14_reg is
  port (clk : in std_logic; d : in t14; q : out t14 := z14);
end entity tc_e14_reg;

architecture rtl of tc_e14_reg is
  alias ff is f [t14 return t14];
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= ff(d);
    end if;
  end process;
end architecture rtl;

library ieee;
use ieee.numeric_std.all;
use work.tc_p14.all;

entity tc_e14_cmb is
  port (d : in t14; y : out integer);
end entity tc_e14_cmb;

architecture comb of tc_e14_cmb is
begin
  y <= idx(d) + f(d, 1);
end architecture comb;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p15.all;

entity tc_e15_reg is
  port (clk : in std_logic; d : in t15; q : out t15 := z15);
end entity tc_e15_reg;

architecture rtl of tc_e15_reg is
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= f(d);
    end if;
  end process;
end architecture rtl;

library ieee;
use ieee.numeric_std.all;
use work.tc_p15.all;

entity tc_e15_cmb is
  port (d : in t15; y : out integer);
end entity tc_e15_cmb;

architecture comb of tc_e15_cmb is
begin
  process (all)
    variable a : integer;
    variable b : boolean;
  begin
    parts(d, a, b);
    if b then
      y <= a + f(d, 1);
    else
      y <= f(d, 1) + a;
    end if;
  end process;
end architecture comb;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p16.all;

entity tc_e16_reg is
  port (clk : in std_logic; d : in t16; q : out t16 := z16);
end entity tc_e16_reg;

architecture rtl of tc_e16_reg is
  use work.tc_p01.idx;
  signal dbg : integer := 0;
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= f(d);
      dbg <= idx(d);
    end if;
  end process;
end architecture rtl;

library ieee;
use ieee.numeric_std.all;
use work.tc_p16.all;

entity tc_e16_cmb is
  port (d : in t16; y : out integer);
end entity tc_e16_cmb;

architecture comb of tc_e16_cmb is
begin
  y <= idx(d) + f(d, 1);
end architecture comb;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p17.all;

entity tc_e17_reg is
  port (clk : in std_logic; d : in t17; q : out t17 := z17);
end entity tc_e17_reg;

architecture rtl of tc_e17_reg is
  alias ff is f [t17 return t17];
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= ff(d);
    end if;
  end process;
end architecture rtl;

library ieee;
use ieee.numeric_std.all;
use work.tc_p17.all;

entity tc_e17_cmb is
  port (d : in t17; y : out integer);
end entity tc_e17_cmb;

architecture comb of tc_e17_cmb is
begin
  process (all)
    variable a : integer;
    variable b : boolean;
  begin
    parts(d, a, b);
    if b then
      y <= a + f(d, 1);
    else
      y <= f(d, 1) + a;
    end if;
  end process;
end architecture comb;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p18.all;

entity tc_e18_reg is
  port (clk : in std_logic; d : in t18; q : out t18 := z18);
end entity tc_e18_reg;

architecture rtl of tc_e18_reg is
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= f(d);
    end if;
  end process;
end architecture rtl;

library ieee;
use ieee.numeric_std.all;
use work.tc_p18.all;

entity tc_e18_cmb is
  port (d : in t18; y : out integer);
end entity tc_e18_cmb;

architecture comb of tc_e18_cmb is
begin
  y <= idx(d) + f(d, 1);
end architecture comb;

library ieee;
use ieee.std_logic_1164.all;
use work.tc_p19.all;

entity tc_e19_reg is
  port (clk : in std_logic; d : in t19; q : out t19 := z19);
end entity tc_e19_reg;

architecture rtl of tc_e19_reg is
  use work.tc_p04.idx;
  signal dbg : integer := 0;
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= f(d);
      dbg <= idx(d);
    end if;
  end process;
end architecture rtl;

library ieee;
use ieee.numeric_std.all;
use work.tc_p19.all;

entity tc_e19_cmb is
  port (d : in t19; y : out integer);
end entity tc_e19_cmb;

architecture comb of tc_e19_cmb is
begin
  process (all)
    variable a : integer;
    variable b : boolean;
  begin
    parts(d, a, b);
    if b then
      y <= a + f(d, 1);
    else
      y <= f(d, 1) + a;
    end if;
  end process;
end architecture comb;

-- Testbench: signals and entities for every family type, then scenario procedures.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.tc_chk.all;
use work.tc_misc.all;
use work.tc_p00.all;
use work.tc_w00;
use work.tc_p01.all;
use work.tc_w01;
use work.tc_p02.all;
use work.tc_w02;
use work.tc_p03.all;
use work.tc_w03;
use work.tc_p04.all;
use work.tc_w04;
use work.tc_p05.all;
use work.tc_w05;
use work.tc_p06.all;
use work.tc_w06;
use work.tc_p07.all;
use work.tc_w07;
use work.tc_p08.all;
use work.tc_w08;
use work.tc_p09.all;
use work.tc_w09;
use work.tc_p10.all;
use work.tc_w10;
use work.tc_p11.all;
use work.tc_w11;
use work.tc_p12.all;
use work.tc_w12;
use work.tc_p13.all;
use work.tc_w13;
use work.tc_p14.all;
use work.tc_w14;
use work.tc_p15.all;
use work.tc_w15;
use work.tc_p16.all;
use work.tc_w16;
use work.tc_p17.all;
use work.tc_w17;
use work.tc_p18.all;
use work.tc_w18;
use work.tc_p19.all;
use work.tc_w19;

entity text_closure_tb is
end entity text_closure_tb;

architecture tb of text_closure_tb is
  signal clk : std_logic := '0';
  signal d00, q00, c00 : t00 := z00;
  signal y00 : integer := 0;
  signal d01, q01, c01 : t01 := z01;
  signal y01 : integer := 0;
  signal d02, q02, c02 : t02 := z02;
  signal y02 : integer := 0;
  signal d03, q03, c03 : t03 := z03;
  signal y03 : integer := 0;
  signal d04, q04, c04 : t04 := z04;
  signal y04 : integer := 0;
  signal d05, q05, c05 : t05 := z05;
  signal y05 : integer := 0;
  signal d06, q06, c06 : t06 := z06;
  signal y06 : integer := 0;
  signal d07, q07, c07 : t07 := z07;
  signal y07 : integer := 0;
  signal d08, q08, c08 : t08 := z08;
  signal y08 : integer := 0;
  signal d09, q09, c09 : t09 := z09;
  signal y09 : integer := 0;
  signal d10, q10, c10 : t10 := z10;
  signal y10 : integer := 0;
  signal d11, q11, c11 : t11 := z11;
  signal y11 : integer := 0;
  signal d12, q12, c12 : t12 := z12;
  signal y12 : integer := 0;
  signal d13, q13, c13 : t13 := z13;
  signal y13 : integer := 0;
  signal d14, q14, c14 : t14 := z14;
  signal y14 : integer := 0;
  signal d15, q15, c15 : t15 := z15;
  signal y15 : integer := 0;
  signal d16, q16, c16 : t16 := z16;
  signal y16 : integer := 0;
  signal d17, q17, c17 : t17 := z17;
  signal y17 : integer := 0;
  signal d18, q18, c18 : t18 := z18;
  signal y18 : integer := 0;
  signal d19, q19, c19 : t19 := z19;
  signal y19 : integer := 0;

  procedure s_literals is
    variable u : unsigned(7 downto 0);
    variable s : signed(7 downto 0);
    variable v : std_logic_vector(7 downto 0);
    variable bv : bit_vector(7 downto 0);
  begin
    v := x"A5";
    u := x"A5";
    s := x"A5";
    bv := x"A5";
    ck(v = "10100101", 1);
    ck(u = 165, 2);
    ck(s = -91, 3);
    ck(bv = b"1010_0101", 4);
    ck(to_integer(unsigned'(o"17")) = 15, 5);
    ck(to_integer(signed'(8sx"F0")) = -16, 6);
    ck(to_integer(unsigned'(12ux"0A5")) = 165, 7);
    ck(to_integer(unsigned'(10d"500")) = 500, 8);
    ck(v(7 downto 4) = x"A", 9);
    ck((v(3 downto 0) & v(7 downto 4)) = x"5A", 10);
    ck(('1' & v(6 downto 0)) = x"A5", 11);
    ck(unsigned(v) + 1 = 166, 12);
    ck(1 + unsigned(v) = 166, 13);
    ck(resize(u, 9) + resize(u, 9) = 330, 14);
    ck(shift_left(u, 1) = x"4A", 15);
    ck((u sll 1) = x"4A" and (u srl 4) = x"0A", 16);
    ck((u rol 4) = x"5A" and (u ror 4) = x"5A", 17);
    ck((and v) = '0', 18);
    ck((or v) = '1', 19);
    ck((xor v) = '0', 20);
    ck(v ?= x"A5", 21);
    ck(std_match(v, "1-1-0101"), 22);
    ck(v(0) = '1' and v(1) = '0' and v(7) = '1', 23);
    ck(s < 0 and s < -1 and s + 91 = 0, 24);
    ck(to_integer(s) = -91 and to_integer(abs s) = 91, 25);
    ck(std_logic_vector(u) = v and unsigned(std_logic_vector'(x"0F")) = 15, 26);
    ck(to_bitvector(v) = bv and to_stdlogicvector(bv) = v, 27);
    ck(to_01(unsigned'("1X0")) = "000", 28);
    ck(unsigned'("101") * 2 = 10 and unsigned'("101") / 2 = 2 and unsigned'("101") rem 2 = 1, 29);
    ck(to_unsigned(44, 8) = x"2C" and to_signed(-3, 4) = "1101", 30);
    ck(u > 100 and 100 < u and u >= 165 and u /= 0, 31);
  end procedure;

  procedure s_concat is
    variable s : string(1 to 5);
    variable v : std_logic_vector(3 downto 0);
  begin
    s := "ab" & 'c' & "de";
    ck(s = "abcde", 40);
    ck(string'('a' & 'b') = "ab", 41);
    ck((character'('a') & 'b') = "ab", 42);
    ck(integer'image(42) & "x" = "42x", 43);
    ck((string'("") & "x") = "x", 44);
    ck((std_logic_vector'("10") & '1') = "101", 45);
    v := '1' & "01" & '0';
    ck(v = "1010", 46);
    v := v(2 downto 0) & v(3);
    ck(v = "0101", 47);
    ck(first_of(string'("abc")) = 'a', 48);
    ck(first_of(std_logic_vector'("1010")) = '1', 49);
    ck(first_of(unsigned'("0101")) = '0', 50);
    ck(first_of(bit_vector'("10")) = '1', 51);
    ck(first_of(ivec'(7, 8)) = 7, 52);
    ck(first_of(bvec'(true, false)), 53);
    ck(len_of(ivec'(1, 2, 3)) = 3, 54);
    ck(len_of(string'("hello")) = 105, 55);
    ck(len_of(seg_vec'(unit_seg, unit_seg)) = 202, 56);
    ck(first_of(string'("x") & "yz") = 'x', 57);
    ck(len_of(ivec'(1, 2) & ivec'(1 => 3)) = 3, 58);
    ck(len_of(ivec'(1 => 1) & 2 & 3) = 3, 59);
    ck(len_of(0 & ivec'(1 => 1)) = 2, 60);
    report "concat " & integer'image(3) & ' ' & "ok" & character'('!') severity note;
  end procedure;

  procedure s_enums is
    variable a : c1 := red;
    variable b : c2 := red;
    variable c : c3 := 'g';
    variable n : natural := 0;
  begin
    ck(a = red, 70);
    ck(b = red, 71);
    ck(c < 'b' and c > 'r', 72);
    ck(c1'(blue) = blue1 and blue2 = c2'(blue), 73);
    ck(name_of(c1'(red)) = "c1:red", 74);
    ck(name_of(c2'(white)) = "c2:white", 75);
    ck(name_of(bit'('1')) = "bit" and name_of(std_ulogic'('1')) = "sl" and name_of(character'('1')) = "char", 76);
    ck(name_of(c3'('r')) = "c3:'r'", 77);
    ck(name_of(true) = "bool" and name_of(1) = "int" and name_of(1.0) = "real" and name_of(1 ns) = "time", 78);
    ck(a + 1 = green and b + 2 = blue and name_c1(a + 2) = "c1:blue", 79);
    case a is
      when red => n := 1;
      when green | blue => n := 2;
    end case;
    ck(n = 1, 80);
    case c1'(blue) is
      when red => n := 3;
      when green => n := 4;
      when blue => n := 5;
    end case;
    ck(n = 5, 81);
    case c is
      when 'r' => n := 6;
      when 'g' => n := 7;
      when 'b' => n := 8;
    end case;
    ck(n = 7, 82);
    ck(c1'pos(blue) = 2 and c2'val(3) = white and c1'succ(red) = green, 83);
    ck(c3'value("'g'") = 'g' and c2'value("yellow") = yellow, 84);
    ck(c1'left = red and c2'right = white and c3'high = 'b', 85);
    ck(c1'pos(c1'high) + c2'pos(c2'high) = 5, 86);
    for k in c1 loop
      n := n + c1'pos(k);
    end loop;
    ck(n = 10, 87);
    for k in c2 range yellow to white loop
      n := n + 1;
    end loop;
    ck(n = 13, 88);
  end procedure;

  procedure s_aggr is
    variable v : std_logic_vector(7 downto 0) := (7 => '1', 3 downto 1 => '1', others => '0');
    variable bv : bits_t(0 to 3) := ('1', '0', '1', others => '0');
    variable iv : ivec(1 to 4) := (1 => 10, 2 => 20, others => 0);
    variable p : pt := (x => 1, y => 2);
    variable sg : seg := unit_seg;
    variable sv : seg_vec(0 to 1) := (others => unit_seg);
    variable gr : grid := ((1, 2, 3), (4, 5, 6));
    constant k : ivec := (3, 4, 5);
  begin
    ck(v = x"8E", 100);
    v := (0 | 2 => '1', others => '0');
    ck(v = x"05", 101);
    ck(bv = bits_t'('1', '0', '1', '0'), 102);
    ck(iv = ivec'(10, 20, 0, 0), 103);
    ck(p = (1, 2), 104);
    ck(p + (3, 4) = (4, 6), 105);
    ck(plus_pt(p, p) = (2, 4), 106);
    ck(-p = (-1, -2), 107);
    ck(p < (2, 0) and not (p < origin), 108);
    ck(2 * p = (2, 4) and 2.0 * p = (2, 4) and 1.5 * p = (2, 3), 109);
    ck((origin & p) = ivec'(0, 0, 1, 2), 110);
    ck(sg.b.x = 1 and sg.tag = green, 111);
    sg := (a => origin, b => (5, 6), tag => blue);
    ck(sg.tag = blue and sg.b.y = 6, 112);
    sg := (origin, p, red);
    ck(sg.tag = red and sg.b = p, 113);
    sg := (a => origin, b => p, tag => c1'(red));
    ck(sg.a = origin, 114);
    ck(sv(1).tag = green, 115);
    sv := (0 => unit_seg, 1 => (origin, p, blue));
    ck(sv(1).tag = blue and sv(0).tag = green, 116);
    ck(gr(1, 2) = 6 and gr(0, 0) = 1, 117);
    gr := (others => (others => 7));
    ck(gr(0, 0) = 7 and gr(1, 2) = 7, 118);
    gr := (0 => (1, 2, 3), 1 => (others => 9));
    ck(gr(1, 0) = 9 and gr(0, 2) = 3, 119);
    ck(k'length = 3 and k'left = 0 and k(2) = 5, 120);
    ck(iv'left = 1 and iv'right = 4 and iv'ascending, 121);
    ck(v'length = 8 and v'high = 7 and v'low = 0 and not v'ascending, 122);
  end procedure;

  procedure s_ops is
    constant a : ivec(0 to 2) := (1, 2, 3);
    constant b : ivec(0 to 2) := (1, 2, 4);
    constant p : bvec(0 to 2) := (true, false, true);
    constant q : bvec(0 to 2) := (true, true, false);
    constant bb : bits_t(0 to 3) := ('1', '0', '1', '1');
    constant d : duration_t := 3 kilo;
  begin
    ck(a < b and a /= b and not (a = b) and b > a and a <= b, 130);
    ck((a & b) = ivec'(1, 2, 3, 1, 2, 4), 131);
    ck(a + 1 = ivec'(2, 3, 4), 132);
    ck((p and q) = bvec'(true, false, false), 133);
    ck((p or q) = bvec'(true, true, true), 134);
    ck((p xor q) = bvec'(false, true, true), 135);
    ck((not p) = bvec'(false, true, false), 136);
    ck((p nand q) = bvec'(false, true, true) and (p nor q) = bvec'(false, false, false), 137);
    ck((bb sll 1) = bits_t'('0', '1', '1', '0') and (bb srl 1) = bits_t'('0', '1', '0', '1'), 138);
    ck((bb rol 1) = bits_t'('0', '1', '1', '1') and (bb ror 1) = bits_t'('1', '1', '0', '1'), 139);
    ck(d + 5 = 3005 tick, 140);
    ck(d / (1 kilo) = 3 and d * 2 = 6 kilo and 2 * d = 6 kilo and d / 3 = 1 kilo, 141);
    ck(d > 2 kilo and abs d = d and d - 1 kilo = 2 kilo and d mod (2 kilo) = 1 kilo, 142);
    ck(2 * 3 ns = 6 ns and 3 ns * 2 = 6 ns and 2.5 * 4 ns = 10 ns, 143);
    ck(1 sec / 10 ms = 100 and 10 ns / 4 = 2500 ps and 5 ns mod 2 ns = 1 ns, 144);
    ck(-(3 ns) = -3 ns and abs (-3 ns) = 3 ns, 145);
    ck(2 ** 8 - 1 = 255 and 7 / 2 = 3 and -7 / 2 = -3 and (-7) mod 3 = 2 and (-7) rem 3 = -1, 146);
    ck(abs (-5) = 5 and real(7) / 2.0 = 3.5 and 2.0 ** 3 = 8.0, 147);
    ck(integer(2.6) = 3 and integer(-2.5) = -3 and integer(2.4) = 2, 148);
    ck(pick(1, 2) = 3 and pick(1.5, 2.0) = 3.0 and pick(5, 1.5) = 3.5 and pick(5.5, 1) = 4.5, 149);
    ck(area = 1.0 and area(3) = 3.0 and area(h => 4) = 4.0 and area(2, 3) = 6.0, 150);
    ck(area(scale => 2.0, w => 5) = 10.0 and area(2.0) = 12.0 and area(w => 2, h => 2, scale => 0.5) = 2.0, 151);
    ck(minimum(3, 4) = 3 and maximum(2.5, 1.5) = 2.5 and minimum(2 ns, 3 ns) = 2 ns, 152);
    ck(natural'high = integer'high and time'low < 0 ns and character'pos(character'high) = 255, 153);
    ck(boolean'pos(true) = 1 and bit'val(1) = '1' and std_ulogic'pos('Z') = 4, 154);
  end procedure;

  procedure s_calls is
    variable i1, i2 : integer;
    variable b1 : boolean;
  begin
    pr(3, i1);
    ck(i1 = 6, 160);
    pr(3, b1);
    ck(b1, 161);
    pr(red, i2);
    ck(i2 = 0, 162);
    pr(blue, i2);
    ck(i2 = 2, 163);
    pr(ivec'(1, 2, 3), i2);
    ck(i2 = 3, 164);
    pr((1, 2, 3, 4), i2);
    ck(i2 = 4, 165);
    pr(y => i1, x => 4);
    ck(i1 = 8, 166);
    ck(nest(3) = 40 and nest(3, 4) = 304, 167);
    ck(twice(2) = 4 and twice(2.0) = 4.0, 168);
    ck(len_of(twice(ivec'(1, 2))) = 4, 169);
    ck(twice(twice(3)) = 12 and twice(twice(1.5)) = 6.0, 170);
    ck("+"(2, 3) = 5 and "+"(origin, origin) = origin, 171);
    ck(work.tc_misc."+"(origin, (1, 1)) = (1, 1), 172);
    ck(ieee.numeric_std."+"(to_unsigned(2, 4), 1) = 3, 173);
    ck(twice(ivec'(5, 6))(2) = 5, 174);
    ck(first_of(twice(ivec'(9, 8))) = 9, 175);
    ck(integer'image(twice(21)) = "42" and real'image(twice(0.5)) = real'image(1.0), 176);
  end procedure;

  procedure s_generated is
    variable v04 : t04;
    variable v05 : t05;
    variable v06 : t06 := conv;
    variable v07 : t07;
    variable a : integer;
    variable b : boolean;
  begin
    v05 := conv;
    v04 := f("abc");
    v07 := from_idx(77);
    ck(idx(v05) = 1 and idx(t05'(conv)) = idx(v05), 200);
    ck(idx(v04) = 7, 201);
    ck(idx(v06) = 6 and idx(t06'(from_idx(6))) = idx(v06), 202);
    ck(idx(v07) = 77, 203);
    bump(v05);
    ck(idx(v05) = 3, 204);
    parts(v07, a, b);
    ck(a = idx(v07) and (b = (a mod 2 = 0)), 205);
    ck(g(v05, v04) = idx(v05) * 1000 + idx(v04), 206);
    ck(f(v04, 2) = idx(v04) + 2 and f(v05, 3) = idx(v05) + 3, 207);
    ck(idx(f(f(v06))) = idx(f(t06'(f(v06)))), 208);
    ck(f05(v05) = f(v05) and f04(v04) = f(v04), 209);
  end procedure;

begin
  p_clk : process
  begin
    for i in 1 to 12 loop
      wait for 5 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
    end loop;
    wait;
  end process p_clk;

  r00 : entity work.tc_e00_reg port map (clk, d00, q00);
  m00 : entity work.tc_e00_cmb port map (d00, y00);
  h00 : entity work.tc_e00_chain port map (clk, d00, c00);
  r01 : entity work.tc_e01_reg port map (clk, d01, q01);
  m01 : entity work.tc_e01_cmb port map (d01, y01);
  h01 : entity work.tc_e01_chain port map (clk, d01, c01);
  r02 : entity work.tc_e02_reg port map (clk, d02, q02);
  m02 : entity work.tc_e02_cmb port map (d02, y02);
  h02 : entity work.tc_e02_chain port map (clk, d02, c02);
  r03 : entity work.tc_e03_reg port map (clk, d03, q03);
  m03 : entity work.tc_e03_cmb port map (d03, y03);
  h03 : entity work.tc_e03_chain port map (clk, d03, c03);
  r04 : entity work.tc_e04_reg port map (clk, d04, q04);
  m04 : entity work.tc_e04_cmb port map (d04, y04);
  h04 : entity work.tc_e04_chain port map (clk, d04, c04);
  r05 : entity work.tc_e05_reg port map (clk, d05, q05);
  m05 : entity work.tc_e05_cmb port map (d05, y05);
  h05 : entity work.tc_e05_chain port map (clk, d05, c05);
  r06 : entity work.tc_e06_reg port map (clk, d06, q06);
  m06 : entity work.tc_e06_cmb port map (d06, y06);
  h06 : entity work.tc_e06_chain port map (clk, d06, c06);
  r07 : entity work.tc_e07_reg port map (clk, d07, q07);
  m07 : entity work.tc_e07_cmb port map (d07, y07);
  h07 : entity work.tc_e07_chain port map (clk, d07, c07);
  r08 : entity work.tc_e08_reg port map (clk, d08, q08);
  m08 : entity work.tc_e08_cmb port map (d08, y08);
  h08 : entity work.tc_e08_chain port map (clk, d08, c08);
  r09 : entity work.tc_e09_reg port map (clk, d09, q09);
  m09 : entity work.tc_e09_cmb port map (d09, y09);
  h09 : entity work.tc_e09_chain port map (clk, d09, c09);
  r10 : entity work.tc_e10_reg port map (clk, d10, q10);
  m10 : entity work.tc_e10_cmb port map (d10, y10);
  r11 : entity work.tc_e11_reg port map (clk, d11, q11);
  m11 : entity work.tc_e11_cmb port map (d11, y11);
  r12 : entity work.tc_e12_reg port map (clk, d12, q12);
  m12 : entity work.tc_e12_cmb port map (d12, y12);
  r13 : entity work.tc_e13_reg port map (clk, d13, q13);
  m13 : entity work.tc_e13_cmb port map (d13, y13);
  r14 : entity work.tc_e14_reg port map (clk, d14, q14);
  m14 : entity work.tc_e14_cmb port map (d14, y14);
  r15 : entity work.tc_e15_reg port map (clk, d15, q15);
  m15 : entity work.tc_e15_cmb port map (d15, y15);
  r16 : entity work.tc_e16_reg port map (clk, d16, q16);
  m16 : entity work.tc_e16_cmb port map (d16, y16);
  r17 : entity work.tc_e17_reg port map (clk, d17, q17);
  m17 : entity work.tc_e17_cmb port map (d17, y17);
  r18 : entity work.tc_e18_reg port map (clk, d18, q18);
  m18 : entity work.tc_e18_cmb port map (d18, y18);
  r19 : entity work.tc_e19_reg port map (clk, d19, q19);
  m19 : entity work.tc_e19_cmb port map (d19, y19);

  g_scope : for k in 1 to 3 generate
    function nest (x : integer) return integer is
    begin
      return x + k;
    end function;
    signal s : integer := k;
  begin
    assert nest(10) = 10 + k and nest(3, 4) = 304 and s = k report "generate scope" severity failure;
  end generate g_scope;

  p_scope : process
    function twice (x : integer) return integer is
    begin
      return x * 3;
    end function;
    variable pick : integer := 5;
    constant area : integer := 7;
  begin
    ck(twice(2) = 6, 300);
    ck(work.tc_misc.twice(2) = 4, 301);
    ck(twice(2.0) = 4.0, 302);
    ck(pick = 5 and work.tc_misc.pick(1, 2) = 3, 303);
    ck(area = 7 and work.tc_misc.area(2) = 2.0, 304);
    wait;
  end process p_scope;

  p_main : process
  begin
    d00 <= from_idx(3);
    d01 <= from_idx(10);
    d02 <= from_idx(17);
    d03 <= from_idx(24);
    d04 <= from_idx(31);
    d05 <= from_idx(38);
    d06 <= from_idx(45);
    d07 <= from_idx(52);
    d08 <= from_idx(59);
    d09 <= from_idx(66);
    d10 <= from_idx(73);
    d11 <= from_idx(80);
    d12 <= from_idx(87);
    d13 <= from_idx(94);
    d14 <= from_idx(101);
    d15 <= from_idx(108);
    d16 <= from_idx(115);
    d17 <= from_idx(122);
    d18 <= from_idx(129);
    d19 <= from_idx(136);
    s_literals;
    s_concat;
    s_enums;
    s_aggr;
    s_ops;
    s_calls;
    s_generated;
    wait for 55 ns;
    ck(idx(d00) = 3, 1000);
    ck(idx(q00) = 9, 1001);
    ck(y00 = idx(d00) + f(d00, 1), 1002);
    ck(idx(work.tc_w00.twice(d00)) = 6, 1003);
    ck(work.tc_w00.same(q00, f00(d00)), 1004);
    ck(idx(c00) = 27, 1005);
    ck(idx(f00(conv)) = idx(f(t00'(conv))), 1007);
    ck(idx(d01) = 2, 1010);
    ck(idx(q01) = 0, 1011);
    ck(y01 = idx(d01) + f(d01, 1), 1012);
    ck(idx(work.tc_w01.twice(d01)) = 0, 1013);
    ck(work.tc_w01.same(q01, f01(d01)), 1014);
    ck(idx(c01) = 2, 1015);
    ck(g(d01, d00) = idx(d01) * 1000 + idx(d00), 1016);
    ck(idx(f01(conv)) = idx(f(t01'(conv))), 1017);
    ck(idx(d02) = 17, 1020);
    ck(idx(q02) = 34, 1021);
    ck(y02 = idx(d02) + f(d02, 1), 1022);
    ck(idx(work.tc_w02.twice(d02)) = 34, 1023);
    ck(work.tc_w02.same(q02, f02(d02)), 1024);
    ck(idx(c02) = 51, 1025);
    ck(g(d02, d01) = idx(d02) * 1000 + idx(d01), 1026);
    ck(idx(f02(conv)) = idx(f(t02'(conv))), 1027);
    ck(idx(d03) = 24, 1030);
    ck(idx(q03) = 48, 1031);
    ck(y03 = idx(d03) + f(d03, 1), 1032);
    ck(idx(work.tc_w03.twice(d03)) = 48, 1033);
    ck(work.tc_w03.same(q03, f03(d03)), 1034);
    ck(idx(c03) = 96, 1035);
    ck(g(d03, d02) = idx(d03) * 1000 + idx(d02), 1036);
    ck(idx(f03(conv)) = idx(f(t03'(conv))), 1037);
    ck(idx(d04) = 31, 1040);
    ck(idx(q04) = 97, 1041);
    ck(y04 = idx(d04) + f(d04, 1), 1042);
    ck(idx(work.tc_w04.twice(d04)) = 62, 1043);
    ck(work.tc_w04.same(q04, f04(d04)), 1044);
    ck(idx(c04) = 295, 1045);
    ck(g(d04, d03) = idx(d04) * 1000 + idx(d03), 1046);
    ck(idx(f04(conv)) = idx(f(t04'(conv))), 1047);
    ck(idx(d05) = 2, 1050);
    ck(idx(q05) = 0, 1051);
    ck(y05 = idx(d05) + f(d05, 1), 1052);
    ck(idx(work.tc_w05.twice(d05)) = 0, 1053);
    ck(work.tc_w05.same(q05, f05(d05)), 1054);
    ck(idx(c05) = 2, 1055);
    ck(g(d05, d04) = idx(d05) * 1000 + idx(d04), 1056);
    ck(idx(f05(conv)) = idx(f(t05'(conv))), 1057);
    ck(idx(d06) = 45, 1060);
    ck(idx(q06) = 90, 1061);
    ck(y06 = idx(d06) + f(d06, 1), 1062);
    ck(idx(work.tc_w06.twice(d06)) = 90, 1063);
    ck(work.tc_w06.same(q06, f06(d06)), 1064);
    ck(idx(c06) = 135, 1065);
    ck(g(d06, d05) = idx(d06) * 1000 + idx(d05), 1066);
    ck(idx(f06(conv)) = idx(f(t06'(conv))), 1067);
    ck(idx(d07) = 52, 1070);
    ck(idx(q07) = 104, 1071);
    ck(y07 = idx(d07) + f(d07, 1), 1072);
    ck(idx(work.tc_w07.twice(d07)) = 104, 1073);
    ck(work.tc_w07.same(q07, f07(d07)), 1074);
    ck(idx(c07) = 208, 1075);
    ck(g(d07, d06) = idx(d07) * 1000 + idx(d06), 1076);
    ck(idx(f07(conv)) = idx(f(t07'(conv))), 1077);
    ck(idx(d08) = 59, 1080);
    ck(idx(q08) = 185, 1081);
    ck(y08 = idx(d08) + f(d08, 1), 1082);
    ck(idx(work.tc_w08.twice(d08)) = 118, 1083);
    ck(work.tc_w08.same(q08, f08(d08)), 1084);
    ck(idx(c08) = 563, 1085);
    ck(g(d08, d07) = idx(d08) * 1000 + idx(d07), 1086);
    ck(idx(f08(conv)) = idx(f(t08'(conv))), 1087);
    ck(idx(d09) = 2, 1090);
    ck(idx(q09) = 0, 1091);
    ck(y09 = idx(d09) + f(d09, 1), 1092);
    ck(idx(work.tc_w09.twice(d09)) = 0, 1093);
    ck(work.tc_w09.same(q09, f09(d09)), 1094);
    ck(idx(c09) = 2, 1095);
    ck(g(d09, d08) = idx(d09) * 1000 + idx(d08), 1096);
    ck(idx(f09(conv)) = idx(f(t09'(conv))), 1097);
    ck(idx(d10) = 73, 1100);
    ck(idx(q10) = 146, 1101);
    ck(y10 = idx(d10) + f(d10, 1), 1102);
    ck(idx(work.tc_w10.twice(d10)) = 146, 1103);
    ck(work.tc_w10.same(q10, f10(d10)), 1104);
    ck(g(d10, d09) = idx(d10) * 1000 + idx(d09), 1106);
    ck(idx(f10(conv)) = idx(f(t10'(conv))), 1107);
    ck(idx(d11) = 80, 1110);
    ck(idx(q11) = 160, 1111);
    ck(y11 = idx(d11) + f(d11, 1), 1112);
    ck(idx(work.tc_w11.twice(d11)) = 160, 1113);
    ck(work.tc_w11.same(q11, f11(d11)), 1114);
    ck(g(d11, d10) = idx(d11) * 1000 + idx(d10), 1116);
    ck(idx(f11(conv)) = idx(f(t11'(conv))), 1117);
    ck(idx(d12) = 87, 1120);
    ck(idx(q12) = 273, 1121);
    ck(y12 = idx(d12) + f(d12, 1), 1122);
    ck(idx(work.tc_w12.twice(d12)) = 174, 1123);
    ck(work.tc_w12.same(q12, f12(d12)), 1124);
    ck(g(d12, d11) = idx(d12) * 1000 + idx(d11), 1126);
    ck(idx(f12(conv)) = idx(f(t12'(conv))), 1127);
    ck(idx(d13) = 2, 1130);
    ck(idx(q13) = 0, 1131);
    ck(y13 = idx(d13) + f(d13, 1), 1132);
    ck(idx(work.tc_w13.twice(d13)) = 0, 1133);
    ck(work.tc_w13.same(q13, f13(d13)), 1134);
    ck(g(d13, d12) = idx(d13) * 1000 + idx(d12), 1136);
    ck(idx(f13(conv)) = idx(f(t13'(conv))), 1137);
    ck(idx(d14) = 101, 1140);
    ck(idx(q14) = 202, 1141);
    ck(y14 = idx(d14) + f(d14, 1), 1142);
    ck(idx(work.tc_w14.twice(d14)) = 202, 1143);
    ck(work.tc_w14.same(q14, f14(d14)), 1144);
    ck(g(d14, d13) = idx(d14) * 1000 + idx(d13), 1146);
    ck(idx(f14(conv)) = idx(f(t14'(conv))), 1147);
    ck(idx(d15) = 108, 1150);
    ck(idx(q15) = 216, 1151);
    ck(y15 = idx(d15) + f(d15, 1), 1152);
    ck(idx(work.tc_w15.twice(d15)) = 216, 1153);
    ck(work.tc_w15.same(q15, f15(d15)), 1154);
    ck(g(d15, d14) = idx(d15) * 1000 + idx(d14), 1156);
    ck(idx(f15(conv)) = idx(f(t15'(conv))), 1157);
    ck(idx(d16) = 115, 1160);
    ck(idx(q16) = 361, 1161);
    ck(y16 = idx(d16) + f(d16, 1), 1162);
    ck(idx(work.tc_w16.twice(d16)) = 230, 1163);
    ck(work.tc_w16.same(q16, f16(d16)), 1164);
    ck(g(d16, d15) = idx(d16) * 1000 + idx(d15), 1166);
    ck(idx(f16(conv)) = idx(f(t16'(conv))), 1167);
    ck(idx(d17) = 2, 1170);
    ck(idx(q17) = 0, 1171);
    ck(y17 = idx(d17) + f(d17, 1), 1172);
    ck(idx(work.tc_w17.twice(d17)) = 0, 1173);
    ck(work.tc_w17.same(q17, f17(d17)), 1174);
    ck(g(d17, d16) = idx(d17) * 1000 + idx(d16), 1176);
    ck(idx(f17(conv)) = idx(f(t17'(conv))), 1177);
    ck(idx(d18) = 129, 1180);
    ck(idx(q18) = 2, 1181);
    ck(y18 = idx(d18) + f(d18, 1), 1182);
    ck(idx(work.tc_w18.twice(d18)) = 2, 1183);
    ck(work.tc_w18.same(q18, f18(d18)), 1184);
    ck(g(d18, d17) = idx(d18) * 1000 + idx(d17), 1186);
    ck(idx(f18(conv)) = idx(f(t18'(conv))), 1187);
    ck(idx(d19) = 136, 1190);
    ck(idx(q19) = 16, 1191);
    ck(y19 = idx(d19) + f(d19, 1), 1192);
    ck(idx(work.tc_w19.twice(d19)) = 16, 1193);
    ck(work.tc_w19.same(q19, f19(d19)), 1194);
    ck(g(d19, d18) = idx(d19) * 1000 + idx(d18), 1196);
    ck(idx(f19(conv)) = idx(f(t19'(conv))), 1197);
    summary("text_closure");
    report "PASS" severity note;
    wait;
  end process p_main;
end architecture tb;
