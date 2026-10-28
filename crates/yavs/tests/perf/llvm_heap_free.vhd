-- Compiled code keeps the heap objects it found across a deallocation of another object: each
-- call frees a node, and the table of words its methods read through an access value stays found.
entity llvm_heap_free is
end;
architecture a of llvm_heap_free is
  type words_t is array (natural range <>) of integer;
  type words_ptr is access words_t;
  type node_t is record
    v : integer;
  end record;
  type node_ptr is access node_t;
  type store_t is protected
    impure function get(i : natural) return integer;
    procedure churn(i : natural);
  end protected;
  type store_t is protected body
    variable tab : words_ptr := new words_t'(3, 1, 4, 1, 5, 9, 2, 6);
    impure function get(i : natural) return integer is
    begin
      return tab(i mod tab'length);
    end;
    procedure churn(i : natural) is
      variable n : node_ptr;
    begin
      n := new node_t'(v => i);
      deallocate(n);
    end;
  end protected body;
  shared variable store : store_t;
begin
  process
    variable acc : integer := 0;
  begin
    for i in 0 to 199 loop
      store.churn(i);
      acc := (acc + store.get(i) + store.get(i + 1)) mod 1000003;
    end loop;
    report "acc=" & integer'image(acc);
    wait;
  end process;
end;
