import type { SearchCriterion } from "./types";

export const FIELD_OPTIONS: Array<{
  value: SearchCriterion["field"];
  label: string;
}> = [
  { value: "metadata.title.en", label: "Faculty/School/College of" },
  { value: "metadata.Department.en", label: "Department" },
  { value: "metadata.CourseCode.en", label: "Course Code" },
  { value: "metadata.CourseName.en", label: "Course Name" },
  { value: "metadata.YearOfExamination.en", label: "Year of Examination" },
  { value: "metadata.AcademicLevel.en", label: "Academic Level" },
  { value: "metadata.Semester.en", label: "Semester" },
];

export const CONDITION_OPTIONS: Array<{
  value: SearchCriterion["condition"];
  label: string;
}> = [
  { value: "must", label: "Must" },
  { value: "must_not", label: "Must not" },
  { value: "should", label: "Should" },
];

export const OPERATOR_OPTIONS: Array<{
  value: SearchCriterion["operator"];
  label: string;
}> = [
  { value: "contains", label: "Contain" },
  { value: "term", label: "Equal" },
  { value: "terms", label: "Multi values" },
  { value: "regexp", label: "Regex" },
  { value: "range", label: "Range" },
  { value: "starts_with", label: "Starts with" },
  { value: "ends_with", label: "Ends with" },
];

export const DEFAULT_CRITERION: SearchCriterion = {
  field: "metadata.CourseCode.en",
  condition: "must",
  operator: "contains",
  value: "",
  values: [],
};
