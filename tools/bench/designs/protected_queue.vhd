-- A byte queue on pointer tables behind protected types, the way a verification library keeps
-- its queues (#493): a record copied out of a table of records, a case on its enumeration field,
-- tables of access values to the data, thin subprograms in front of the protected methods. The
-- time goes to the calls of the protected methods and the thin subprograms, one byte at a time.
library ieee;
use ieee.std_logic_1164.all;

entity bench_protected_queue is
  generic (N : natural := 4000);
end entity;

architecture sim of bench_protected_queue is
  type mode_t is (internal, external, function_based);
  type storage_t is record
    id     : integer;
    mode   : mode_t;
    length : integer;
  end record;
  type storage_vec_t is array (natural range <>) of storage_t;
  type storage_vec_ptr_t is access storage_vec_t;
  type bytes_t is array (positive range <>) of character;
  type bytes_ptr_t is access bytes_t;
  type bytes_vec_t is array (natural range <>) of bytes_ptr_t;
  type bytes_vec_ptr_t is access bytes_vec_t;
  type ints_t is array (natural range <>) of integer;
  type ints_ptr_t is access ints_t;
  type ints_vec_t is array (natural range <>) of ints_ptr_t;
  type ints_vec_ptr_t is access ints_vec_t;

  type storage_pt is protected
    impure function new_bytes(length : natural) return natural;
    impure function new_ints(length : natural) return natural;
    impure function length(ref : natural) return integer;
    procedure set_byte(ref : natural; index : positive; value : character);
    impure function get_byte(ref : natural; index : positive) return character;
    procedure set_int(ref : natural; index : natural; value : integer);
    impure function get_int(ref : natural; index : natural) return integer;
    procedure resize(ref : natural; length : natural; drop : natural);
  end protected;

  type storage_pt is protected body
    variable idxs   : storage_vec_ptr_t := new storage_vec_t(0 to 3);
    variable bytes  : bytes_vec_ptr_t := new bytes_vec_t(0 to 3);
    variable ints   : ints_vec_ptr_t := new ints_vec_t(0 to 3);
    variable n_idx, n_bytes, n_ints : natural := 0;

    impure function new_bytes(length : natural) return natural is
      variable old : storage_vec_ptr_t;
      variable oldb : bytes_vec_ptr_t;
    begin
      if n_idx > idxs'high then
        old := idxs;
        idxs := new storage_vec_t(0 to 2 * old'length - 1);
        for i in old'range loop idxs(i) := old(i); end loop;
        deallocate(old);
      end if;
      if n_bytes > bytes'high then
        oldb := bytes;
        bytes := new bytes_vec_t(0 to 2 * oldb'length - 1);
        for i in oldb'range loop bytes(i) := oldb(i); end loop;
        deallocate(oldb);
      end if;
      idxs(n_idx) := (id => n_bytes, mode => internal, length => 0);
      bytes(n_bytes) := new bytes_t'(1 to length => character'val(0));
      n_bytes := n_bytes + 1;
      n_idx := n_idx + 1;
      return n_idx - 1;
    end;

    impure function new_ints(length : natural) return natural is
      variable old : storage_vec_ptr_t;
      variable oldi : ints_vec_ptr_t;
    begin
      if n_idx > idxs'high then
        old := idxs;
        idxs := new storage_vec_t(0 to 2 * old'length - 1);
        for i in old'range loop idxs(i) := old(i); end loop;
        deallocate(old);
      end if;
      if n_ints > ints'high then
        oldi := ints;
        ints := new ints_vec_t(0 to 2 * oldi'length - 1);
        for i in oldi'range loop ints(i) := oldi(i); end loop;
        deallocate(oldi);
      end if;
      idxs(n_idx) := (id => n_ints, mode => function_based, length => 0);
      ints(n_ints) := new ints_t'(0 to length - 1 => 0);
      n_ints := n_ints + 1;
      n_idx := n_idx + 1;
      return n_idx - 1;
    end;

    impure function length(ref : natural) return integer is
      variable s : storage_t := idxs(ref);
    begin
      case s.mode is
        when internal       => return bytes(s.id)'length;
        when function_based => return ints(s.id)'length;
        when others         => return abs(s.length);
      end case;
    end;

    procedure set_byte(ref : natural; index : positive; value : character) is
      variable s : storage_t := idxs(ref);
    begin
      case s.mode is
        when internal => bytes(s.id)(index) := value;
        when others   => report "set_byte on a table that holds integers" severity failure;
      end case;
    end;

    impure function get_byte(ref : natural; index : positive) return character is
      variable s : storage_t := idxs(ref);
    begin
      case s.mode is
        when internal => return bytes(s.id)(index);
        when others   => report "get_byte on a table that holds integers" severity failure;
      end case;
      return character'val(0);
    end;

    procedure set_int(ref : natural; index : natural; value : integer) is
      variable s : storage_t := idxs(ref);
    begin
      case s.mode is
        when function_based => ints(s.id)(index) := value;
        when others         => report "set_int on a table that holds bytes" severity failure;
      end case;
    end;

    impure function get_int(ref : natural; index : natural) return integer is
      variable s : storage_t := idxs(ref);
    begin
      case s.mode is
        when function_based => return ints(s.id)(index);
        when others         => report "get_int on a table that holds bytes" severity failure;
      end case;
      return 0;
    end;

    procedure resize(ref : natural; length : natural; drop : natural) is
      variable s : storage_t := idxs(ref);
      variable newp : bytes_ptr_t;
      variable oldp : bytes_ptr_t;
      variable keep : natural := length;
    begin
      oldp := bytes(s.id);
      newp := new bytes_t'(1 to length => character'val(0));
      if keep > oldp'length - drop then
        keep := oldp'length - drop;
      end if;
      for i in 1 to keep loop
        newp(i) := oldp(drop + i);
      end loop;
      bytes(s.id) := newp;
      deallocate(oldp);
    end;
  end protected body;

  shared variable store : storage_pt;

  -- The thin layer in front of the protected methods.
  type ptr_t is record
    ref : integer;
  end record;
  constant null_ptr : ptr_t := (ref => -1);

  impure function new_ptr(n : natural) return ptr_t is
  begin
    return (ref => store.new_bytes(n));
  end;
  impure function new_int_ptr(n : natural) return ptr_t is
  begin
    return (ref => store.new_ints(n));
  end;
  impure function length(p : ptr_t) return integer is
  begin
    return store.length(p.ref);
  end;
  procedure set(p : ptr_t; index : positive; value : character) is
  begin
    store.set_byte(p.ref, index, value);
  end;
  impure function get(p : ptr_t; index : positive) return character is
  begin
    return store.get_byte(p.ref, index);
  end;
  procedure set(p : ptr_t; index : natural; value : integer) is
  begin
    store.set_int(p.ref, index, value);
  end;
  impure function get(p : ptr_t; index : natural) return integer is
  begin
    return store.get_int(p.ref, index);
  end;

  type queue_t is record
    meta : ptr_t;
    data : ptr_t;
  end record;
  constant head_idx : natural := 0;
  constant tail_idx : natural := 1;

  impure function new_queue return queue_t is
    variable q : queue_t := (meta => new_int_ptr(2), data => new_ptr(0));
  begin
    set(q.meta, head_idx, 0);
    set(q.meta, tail_idx, 0);
    return q;
  end;

  procedure push(q : queue_t; value : character) is
    variable tail, head : integer;
  begin
    assert q.meta /= null_ptr and q.data /= null_ptr report "push to a null queue" severity failure;
    tail := get(q.meta, tail_idx);
    head := get(q.meta, head_idx);
    if length(q.data) < tail + 1 then
      store.resize(q.data.ref, 2 * length(q.data) + 1, head);
      tail := tail - head;
      head := 0;
      set(q.meta, head_idx, head);
    end if;
    set(q.data, 1 + tail, value);
    set(q.meta, tail_idx, tail + 1);
  end;

  impure function pop(q : queue_t) return character is
    variable head : integer;
    variable value : character;
  begin
    head := get(q.meta, head_idx);
    assert head < get(q.meta, tail_idx) report "pop from an empty queue" severity failure;
    value := get(q.data, 1 + head);
    set(q.meta, head_idx, head + 1);
    return value;
  end;

  impure function size(q : queue_t) return natural is
  begin
    return get(q.meta, tail_idx) - get(q.meta, head_idx);
  end;

  procedure push(q : queue_t; value : string) is
  begin
    for i in value'range loop
      push(q, value(i));
    end loop;
  end;

  impure function pop(q : queue_t; n : natural) return string is
    variable s : string(1 to n);
  begin
    for i in s'range loop
      s(i) := pop(q);
    end loop;
    return s;
  end;
begin
  process
    constant q : queue_t := new_queue;
    variable sum : natural := 0;
    variable v : std_ulogic_vector(255 downto 0) := (others => '1');
  begin
    for round in 1 to N loop
      push(q, "queue entry number " & integer'image(round));
      v(round mod 256) := not v(round mod 256);
      for i in v'range loop
        push(q, std_ulogic'image(v(i))(2));
      end loop;
      while size(q) > 0 loop
        sum := sum + character'pos(pop(q));
      end loop;
    end loop;
    report "protected_queue sum=" & integer'image(sum);
    wait;
  end process;
end architecture;
