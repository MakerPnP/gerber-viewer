#!/bin/sh

args="$*"

# List of directories to process
dirs=("." "demo")

# Iterate through each directory in the list
for dir in "${dirs[@]}"; do
    if [ -d "$dir" ]; then
        pushd "$dir" > /dev/null

        if [ -f "Cargo.toml" ]; then
            echo "running command in '$dir':"
            eval "cargo $args"
        else

            # Run the cargo command in each subdirectory
            for f in *; do
                if [ -d "$f" ]; then
                    if [ -d "$f" ] && [ -f "$f/Cargo.toml" ]; then
                        pushd "$f" > /dev/null
                        echo "running command in '$f':"
                        eval "cargo $args"
                        popd > /dev/null
                    fi
                fi
            done
        fi

        popd > /dev/null
    fi
done
