-- A pointer table behind a protected type: its methods are too big to inline into the compiled
-- code, so the interpreter takes them through arms once the tier's count of `sum` and its wait
-- ran out (`opt/inline/arms.rs`): after that a pass of `sum`'s loop makes one interpreted call
-- (the method `set`) instead of four.
package arm_calls_pkg is
  type mode_t is (internal, external, noted);
  type prot_t is protected
    procedure init(n : natural);
    procedure set_mode(ref : natural; m : mode_t);
    impure function get(ref : natural; index : natural) return integer;
    procedure set(ref : natural; index : natural; value : integer);
  end protected;
  impure function get(ref : natural; index : natural) return integer;
  procedure set(ref : natural; index : natural; value : integer);
  procedure set_mode(ref : natural; m : mode_t);
  procedure init(n : natural);
  impure function sum(ref, n, first : natural) return integer;
end package;
package body arm_calls_pkg is
  type int_vec is array (natural range <>) of integer;
  type int_vec_ptr is access int_vec;
  type vec_arr is array (natural range <>) of int_vec_ptr;
  type vec_arr_ptr is access vec_arr;
  type rec_t is record
    id   : natural;
    mode : mode_t;
    len  : integer;
  end record;
  type rec_arr is array (natural range <>) of rec_t;
  type rec_arr_ptr is access rec_arr;

  impure function note(index : natural) return integer is
  begin
    return -index;
  end;

  type prot_t is protected body
    variable idxs  : rec_arr_ptr;
    variable ptrs  : vec_arr_ptr;
    variable eptrs : vec_arr_ptr;

    procedure init(n : natural) is
    begin
      idxs  := new rec_arr'(0 => (0, internal, n), 1 => (0, external, n));
      ptrs  := new vec_arr'(0 => new int_vec'(0 to n - 1 => 1));
      eptrs := new vec_arr'(0 => new int_vec'(0 to n - 1 => 2));
    end;

    procedure set_mode(ref : natural; m : mode_t) is
    begin
      idxs(ref).mode := m;
    end;

    impure function get(ref : natural; index : natural) return integer is
      variable s : rec_t := idxs(ref);
    begin
      case s.mode is
        when internal => return ptrs(s.id)(index);
        when external => return eptrs(s.id)(index);
        when noted    => return note(index);
      end case;
    end;

    procedure set(ref : natural; index : natural; value : integer) is
      variable s : rec_t := idxs(ref);
    begin
      case s.mode is
        when internal => ptrs(s.id)(index) := value;
        when external => eptrs(s.id)(index) := value;
        when noted    => ptrs(s.id)(index) := value + note(index);
      end case;
    end;
  end protected body;

  shared variable st : prot_t;

  impure function get(ref : natural; index : natural) return integer is
  begin
    return st.get(ref, index);
  end;

  procedure set(ref : natural; index : natural; value : integer) is
  begin
    st.set(ref, index, value);
  end;

  procedure set_mode(ref : natural; m : mode_t) is
  begin
    st.set_mode(ref, m);
  end;

  procedure init(n : natural) is
  begin
    st.init(n);
  end;

  -- Several calls with arms in one block.
  impure function sum(ref, n, first : natural) return integer is
    variable acc : integer := 0;
  begin
    for i in first to n - 1 loop
      set(ref, i, get(ref, i) + i);
      acc := acc + get(ref, i) * 2 + get(1, i);
    end loop;
    assert acc /= integer'low report "sum " & integer'image(acc);
    return acc;
  end;
end package body;

use work.arm_calls_pkg.all;
entity arm_calls is
end;
architecture a of arm_calls is
begin
  process
    variable acc : integer := 0;
  begin
    init(8);
    for k in 0 to 1599 loop
      acc := acc + sum(0, 4, 0);
    end loop;
    report "acc=" & integer'image(acc);
    wait;
  end process;
end;
