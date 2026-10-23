-- difftest: top=null_in_table std=2008 exit=1
-- A null access value met in a table behind a protected method: the first reads work, the
-- read of the table entry nobody allocated stops the run with the null dereference.
entity null_in_table is end;
architecture t of null_in_table is
  type ints_t is array (natural range <>) of integer;
  type ints_ptr_t is access ints_t;
  type table_t is array (natural range <>) of ints_ptr_t;
  type table_ptr_t is access table_t;
  type store_pt is protected
    procedure init;
    impure function get(ref : natural; index : natural) return integer;
  end protected;
  type store_pt is protected body
    variable table : table_ptr_t;
    procedure init is
    begin
      table := new table_t(0 to 3);
      table(0) := new ints_t'(10, 20, 30);
      table(2) := new ints_t'(1, 2);
    end;
    impure function get(ref : natural; index : natural) return integer is
    begin
      return table(ref)(index);
    end;
  end protected body;
  shared variable store : store_pt;
  impure function read(ref : natural; index : natural) return integer is
  begin
    return store.get(ref, index);
  end;
begin
  process
    variable sum : integer := 0;
  begin
    store.init;
    sum := read(0, 2) + read(2, 1);
    report "sum " & integer'image(sum);
    sum := sum + read(1, 0);
    report "unreachable " & integer'image(sum);
    wait;
  end process;
end;
