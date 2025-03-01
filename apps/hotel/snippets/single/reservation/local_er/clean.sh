#!/bin/bash

suffixs=("csv" "log")

for suffix in "${suffixs[@]}"; do
	count=$(ls *."$suffix" 2>/dev/null | wc -l)
	if [ "$count" -eq 0 ]; then
		echo "No $suffix files found to delete"
	else
		echo "There are $count $suffix files. Do you want to delete them? (y/n)"
		read -r response
		if [[ "$response" == "y" ]]; then
			rm *."$suffix"
			echo "$count $suffix files deleted"
		else
			echo "Operation cancelled"
		fi
	fi
done
