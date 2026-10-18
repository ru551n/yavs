-- yavs-test: std=2019
-- expect: E1906 7:5 an alias in a protected type declaration must denote a method
-- LRM 2019 5.6.2 (L15)
package p is
  type outer is protected
    private variable iv : integer;
    alias a is iv;
    procedure go;
  end protected;
end package;
