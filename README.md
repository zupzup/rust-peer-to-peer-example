# rust-peer-to-peer-example

An example for building a simple peer to peer application using Rust

Run with `RUST_LOG=info cargo run` in multiple terminals, preferably in different folders, each containing a different, or empty `recipes.json` file.

Upon starting, press enter to start the application. The application expects a `recipes.json` file in the folder where it's executed.

There are several commands:

* `ls p` - list all peers
* `ls r` - list local recipes
* `ls r all` - list all public recipes from all known peers
* `ls r {peerId}` - list all public recipes from the given peer
* `create r Name|Ingredients|Instructions` - create a new recipe with the given data, the `|` are important as separators
* `publish r {recipeId}` - publish recipe with the given recipe ID
