-- difftest: top=state_composite_once std=2008 ref=hand
-- ref=hand: sim1 reads the bound again (discrepancy `object-range-bound-variable`), so `fill`
-- fails on its whole-array assignment; sim2 agrees with this output.
-- A composite state variable of a protected type body whose constraint reads an earlier state
-- variable: each object elaborates the constraint once, when it is created and the body's
-- declarations are elaborated in order (LRM 14.4.2.1), so a later change of the variable does
-- not change the composite's bounds (#1180).
package state_composite_once_c is
  type counter is protected
    impure function bump return integer;
  end protected;
end package;
package body state_composite_once_c is
  type counter is protected body
    variable n : integer := 0;
    impure function bump return integer is
    begin
      n := n + 1;
      return n;
    end function;
  end protected body;
end package body;
use work.state_composite_once_c.all;
package state_composite_once_p is
  shared variable c : counter;
  type rec_t is record
    tag  : integer;
    data : bit_vector;
  end record;
  type inner_t is protected
    impure function info return string;
  end protected;
  type buf_t is protected
    procedure set_k (x : integer);
    impure function info return string;
    procedure fill (b : bit);
    impure function sum return integer;
    impure function early return integer;
    impure function fresh return string;
  end protected;
end package;
package body state_composite_once_p is
  type inner_t is protected body
    variable m : natural := c.bump + 1;
    variable w : bit_vector(m downto 1) := (others => '1');
    impure function info return string is
    begin
      return "m=" & integer'image(m) & " w=" & to_string(w);
    end function;
  end protected body;
  type buf_t is protected body
    -- A method called before `v` exists.
    impure function early return integer is
    begin
      return 3;
    end function;
    variable k : natural := early + c.bump;
    variable v : bit_vector(0 to k);
    variable iv : integer_vector(1 to k) := (others => 7);
    variable d : bit_vector(k + 1 downto 2);
    variable r : rec_t(data(0 to k - 1));
    variable len : integer := v'length + iv'length;
    -- A constant takes its bounds from an initial value that reads the object.
    constant u : bit_vector := (0 to k => '1');
    constant nm : string := "buf" & integer'image(k);
    variable sub : inner_t;
    procedure set_k (x : integer) is
    begin
      k := x;
    end procedure;
    impure function info return string is
      -- A nested subprogram reads the object's bounds too.
      impure function bounds return string is
      begin
        return integer'image(v'left) & " to " & integer'image(v'right);
      end function;
    begin
      return "k=" & integer'image(k) & " v=" & bounds & " v'length=" & integer'image(v'length)
        & " iv'length=" & integer'image(iv'length) & " iv(1)=" & integer'image(iv(1))
        & " d=" & integer'image(d'left) & " downto " & integer'image(d'right)
        & " r.data'length=" & integer'image(r.data'length) & " len=" & integer'image(len)
        & " u'length=" & integer'image(u'length) & " " & nm
        & " " & sub.info;
    end function;
    procedure fill (b : bit) is
      variable t : bit_vector(v'range);
    begin
      for i in v'range loop
        v(i) := b;
      end loop;
      v(v'right) := '0';
      t := v;
      v := t;
      r.data := (others => b);
      r.tag := r.data'length;
      iv(iv'right) := iv'right;
    end procedure;
    impure function sum return integer is
      variable s : integer := 0;
    begin
      for i in v'range loop
        if v(i) = '1' then
          s := s + 1;
        end if;
      end loop;
      for i in iv'range loop
        s := s + iv(i);
      end loop;
      return s + r.tag;
    end function;
    -- An object of its own type created in a method: its own bounds, `self`'s stay.
    impure function fresh return string is
      variable o : buf_t;
    begin
      return "fresh " & o.info & " self v'length=" & integer'image(v'length);
    end function;
  end protected body;
end package body;
use work.state_composite_once_p.all;
entity state_composite_once is
end entity;
architecture a of state_composite_once is
  shared variable sa : buf_t;
  shared variable sb : buf_t;
begin
  process
    variable lp : buf_t;
  begin
    report "a " & sa.info;
    report "b " & sb.info;
    report "l " & lp.info;
    sa.set_k(2);
    sa.fill('1');
    report "a " & sa.info & " sum=" & integer'image(sa.sum);
    lp.fill('1');
    report "l sum=" & integer'image(lp.sum);
    report sb.fresh;
    report "b " & sb.info;
    wait;
  end process;
end architecture;
