-- difftest: top=sigrec_image
-- A whole record read through ports and hazards around it (#1065): every value a process gathers must be
-- the current one, whatever made it so: a collapsed in port of a record and of a part of an array of
-- records, a port with a conversion, two fields of a formal tied to one actual, force and release of the
-- actual and of a leaf, a slice of an array of records, an array of records read whole and in parts,
-- records with integer, real, enumeration and null leaves, and two record types over one signal.
library ieee;
use ieee.std_logic_1164.all;

package sigrec_image_p is
  type col_t is (red, green, blue);
  type rec_t is record
    a : std_ulogic;
    v : std_ulogic_vector(9 downto 0);
    n : integer;
    e : col_t;
    x : real;
    t : time;
    z : std_ulogic_vector(0 downto 1);
    w : std_ulogic_vector(69 downto 0);
    s : string(1 to 3);
  end record;
  constant init : rec_t := ('0', (others => '0'), 0, red, 0.0, 0 ns, "", (others => '0'), "abc");
  type arr_t is array (natural range <>) of rec_t;
  function digest(r : rec_t) return integer;
  type pair_t is record
    p, q : std_ulogic_vector(3 downto 0);
  end record;
  function to_pair(v : std_ulogic_vector(7 downto 0)) return pair_t;
  function to_vec(p : pair_t) return std_ulogic_vector;
end package;

package body sigrec_image_p is
  function digest(r : rec_t) return integer is
    variable d : integer := r.n mod 1009;
  begin
    if r.a = '1' then d := d + 1; end if;
    for i in r.v'range loop
      d := (d * 3) mod 65521;
      if r.v(i) = '1' then d := d + 1; elsif r.v(i) = 'Z' then d := d + 2; end if;
    end loop;
    d := d + col_t'pos(r.e) * 7;
    d := d + integer(r.x * 2.0) + (r.t / 1 ns) mod 13;
    if r.w(69) = '1' then d := d + 16; end if;
    if r.w(0) = '1' then d := d + 32; end if;
    if r.s(2) = 'x' then d := d + 64; end if;
    return d;
  end function;
  function to_pair(v : std_ulogic_vector(7 downto 0)) return pair_t is
  begin
    return (v(7 downto 4), v(3 downto 0));
  end function;
  function to_vec(p : pair_t) return std_ulogic_vector is
  begin
    return p.p & p.q;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.sigrec_image_p.all;

entity img_in is
  port (i : in rec_t; o : out integer := 0);
end entity;
architecture a of img_in is
begin
  process (i)
    variable v : rec_t;
  begin
    v := i;
    o <= digest(v);
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use work.sigrec_image_p.all;

entity img_pair is
  port (i : in pair_t; o : out std_ulogic_vector(7 downto 0));
end entity;
architecture a of img_pair is
begin
  process (i)
    variable v : pair_t;
  begin
    v := i;
    o <= to_vec(v);
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use work.sigrec_image_p.all;

entity sigrec_image is
end entity;

architecture t of sigrec_image is
  signal clk : bit := '0';
  signal step : integer := 0;
  signal r, ff : rec_t := init;
  signal ar : arr_t(0 to 3) := (others => init);
  signal da, db, dc, dd, de : integer := 0;
  signal raw : std_ulogic_vector(7 downto 0) := x"00";
  signal back, tied : std_ulogic_vector(7 downto 0);
  signal one : std_ulogic_vector(3 downto 0) := "0000";
begin
  clk <= not clk after 5 ns when step < 200;

  count : process (clk)
  begin
    if clk = '1' then step <= step + 1; end if;
  end process;

  reg : process (clk)
    variable v : rec_t;
    variable k : integer;
  begin
    if clk = '1' then
      v := r;
      k := step mod 40;
      if k = 1 then v.a := not v.a; end if;
      if k = 3 then v.v(step mod 10) := 'Z'; end if;
      if k = 5 then v.v := (others => '1'); end if;
      if k = 7 then v.n := v.n + step; end if;
      if k = 9 then v.e := col_t'val(step mod 3); end if;
      if k = 11 then v.x := v.x + 0.5; end if;
      if k = 13 then v.t := v.t + 3 ns; end if;
      if k = 15 then v.w(69) := not v.w(69); end if;
      if k = 17 then v.w(0) := not v.w(0); end if;
      if k = 19 then v.s(2) := 'x'; end if;
      if k = 21 then v.s(2) := 'b'; v.v := (others => '0'); end if;
      r <= v;
      ar(step mod 4) <= v;
      raw <= raw(6 downto 0) & not raw(7);
      if k = 2 then one <= not one; end if;
    end if;
  end process;

  -- The same value through a collapsed port, a part of an array of records and a whole array.
  u0 : entity work.img_in port map (i => r, o => da);
  u1 : entity work.img_in port map (i => ar(2), o => db);

  whole : process (ar)
    variable v : arr_t(0 to 3);
    variable d : integer := 0;
  begin
    v := ar;
    d := 0;
    for i in v'range loop d := d + digest(v(i)); end loop;
    dc <= d;
  end process;

  part : process (ar, step)
    variable v : rec_t;
    variable s : arr_t(1 to 2);
  begin
    v := ar(step mod 4);
    s := ar(1 to 2);
    dd <= digest(v) + digest(s(2));
  end process;

  -- A conversion on the actual, the conversions back and two fields tied to one actual.
  u2 : entity work.img_pair port map (i => to_pair(raw), o => back);
  u3 : entity work.img_pair port map (i.p => one, i.q => one, o => tied);

  -- Force and release of a whole record and of a leaf, seen by a gather of the signal and of a
  -- port on it.
  forcer : process
  begin
    wait until step = 60;
    r.v <= force "1010101010";
    wait until step = 66;
    r.v <= release;
    wait until step = 90;
    r <= force ('1', "0011001100", 77, blue, 1.5, 9 ns, "", (others => '1'), "xxx");
    wait until step = 95;
    r <= release;
    wait;
  end process;

  second : process (r)
    variable v : rec_t;
  begin
    v := r;
    de <= digest(v);
  end process;

  rep : process (clk)
    variable last : integer := -1;
    variable now_d : integer;
  begin
    if clk = '0' then
      now_d := da * 7 + db * 5 + dc * 3 + dd + de;
      if now_d /= last then
        report integer'image(step) & " " & integer'image(da) & " " & integer'image(db) & " " & integer'image(dc)
          & " " & integer'image(dd) & " " & integer'image(de) & " " & to_hstring(back) & " " & to_hstring(tied);
        last := now_d;
      end if;
    end if;
  end process;
end architecture;
