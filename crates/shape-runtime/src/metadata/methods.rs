//! Method metadata for Column and other generic types

use super::types::MethodInfo;

/// Get column methods
pub fn column_methods() -> Vec<MethodInfo> {
    vec![
        // Implemented methods
        MethodInfo {
            name: "shift".to_string(),
            signature: "shift(periods: Number) -> Column".to_string(),
            description: "Shifts the series by specified periods".to_string(),
            return_type: "Column".to_string(),
            implemented: true,
        },
        MethodInfo {
            name: "rolling".to_string(),
            signature: "rolling(window: Number) -> RollingWindow".to_string(),
            description: "Creates a rolling window over the series".to_string(),
            return_type: "RollingWindow".to_string(),
            implemented: true,
        },
        MethodInfo {
            name: "mean".to_string(),
            signature: "mean() -> Number".to_string(),
            description: "Calculates the mean of the series".to_string(),
            return_type: "Number".to_string(),
            implemented: true,
        },
        MethodInfo {
            name: "sum".to_string(),
            signature: "sum() -> Number".to_string(),
            description: "Calculates the sum of the series".to_string(),
            return_type: "Number".to_string(),
            implemented: true,
        },
        MethodInfo {
            name: "min".to_string(),
            signature: "min() -> Number".to_string(),
            description: "Finds the minimum value in the series".to_string(),
            return_type: "Number".to_string(),
            implemented: true,
        },
        MethodInfo {
            name: "max".to_string(),
            signature: "max() -> Number".to_string(),
            description: "Finds the maximum value in the series".to_string(),
            return_type: "Number".to_string(),
            implemented: true,
        },
        MethodInfo {
            name: "first".to_string(),
            signature: "first() -> Number".to_string(),
            description: "Gets the first value in the series".to_string(),
            return_type: "Number".to_string(),
            implemented: true,
        },
        MethodInfo {
            name: "last".to_string(),
            signature: "last() -> Number".to_string(),
            description: "Gets the last value in the series".to_string(),
            return_type: "Number".to_string(),
            implemented: true,
        },
        MethodInfo {
            name: "diff".to_string(),
            signature: "diff(periods?: Number) -> Column".to_string(),
            description: "Computes differences between consecutive values".to_string(),
            return_type: "Column".to_string(),
            implemented: true,
        },
        MethodInfo {
            name: "pct_change".to_string(),
            signature: "pct_change(periods?: Number) -> Column".to_string(),
            description: "Computes percentage change between values".to_string(),
            return_type: "Column".to_string(),
            implemented: true,
        },
        MethodInfo {
            name: "slice".to_string(),
            signature: "slice(start: Number, end: Number) -> Column".to_string(),
            description: "Extracts a portion of the series".to_string(),
            return_type: "Column".to_string(),
            implemented: true,
        },
        MethodInfo {
            name: "filter".to_string(),
            signature: "filter(predicate: Function) -> Column".to_string(),
            description: "Filters the series using a predicate function".to_string(),
            return_type: "Column".to_string(),
            implemented: true,
        },
        MethodInfo {
            name: "map".to_string(),
            signature: "map(transform: Function) -> Column".to_string(),
            description: "Transforms each value in the series".to_string(),
            return_type: "Column".to_string(),
            implemented: true,
        },
        MethodInfo {
            name: "reduce".to_string(),
            signature: "reduce(reducer: Function, initial?: Any) -> Any".to_string(),
            description: "Reduces the series to a single value".to_string(),
            return_type: "Any".to_string(),
            implemented: true,
        },
        MethodInfo {
            name: "forEach".to_string(),
            signature: "forEach(callback: Function) -> Unit".to_string(),
            description: "Executes a function for each element".to_string(),
            return_type: "Unit".to_string(),
            implemented: true,
        },
        MethodInfo {
            name: "find".to_string(),
            signature: "find(predicate: Function) -> Any".to_string(),
            description: "Finds the first element matching a predicate".to_string(),
            return_type: "Any".to_string(),
            implemented: true,
        },
        MethodInfo {
            name: "some".to_string(),
            signature: "some(predicate: Function) -> Boolean".to_string(),
            description: "Tests if any element matches a predicate".to_string(),
            return_type: "Boolean".to_string(),
            implemented: true,
        },
        MethodInfo {
            name: "every".to_string(),
            signature: "every(predicate: Function) -> Boolean".to_string(),
            description: "Tests if all elements match a predicate".to_string(),
            return_type: "Boolean".to_string(),
            implemented: true,
        },
        // Simulation method - generic event processing
        MethodInfo {
            name: "simulate".to_string(),
            signature: "simulate(handler: Function, config?: Object) -> SimulationResult"
                .to_string(),
            description: "Runs a simulation over the series, calling handler for each element."
                .to_string(),
            return_type: "SimulationResult".to_string(),
            implemented: true,
        },
        // Not yet implemented methods
        MethodInfo {
            name: "stddev".to_string(),
            signature: "stddev() -> Number".to_string(),
            description: "Calculates the standard deviation of the series".to_string(),
            return_type: "Number".to_string(),
            implemented: false,
        },
        MethodInfo {
            name: "groupBy".to_string(),
            signature: "groupBy(keyFn: Function) -> GroupedColumn".to_string(),
            description: "Groups series elements by key".to_string(),
            return_type: "GroupedColumn".to_string(),
            implemented: false,
        },
        MethodInfo {
            name: "resample".to_string(),
            signature: "resample(target: String) -> Column".to_string(),
            description: "Resamples the series to a different frequency/timeframe".to_string(),
            return_type: "Column".to_string(),
            implemented: false,
        },
    ]
}
